//! Mutation checkpoints and layout-driven resize/intersection delivery.
use super::*;
use dom_bindings::html::{Mutation, MutationKind, NodeId};
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GeometryRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl GeometryRect {
    pub fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite() && n.abs() <= 10_000_000.0)
            && self.width >= 0.0
            && self.height >= 0.0
    }
    fn relative(self, viewport: Self) -> Self {
        Self {
            x: self.x - viewport.x,
            y: self.y - viewport.y,
            ..self
        }
    }
    fn intersection(self, b: Self) -> Self {
        let (x, y) = (self.x.max(b.x), self.y.max(b.y));
        Self {
            x,
            y,
            width: ((self.x + self.width).min(b.x + b.width) - x).max(0.0),
            height: ((self.y + self.height).min(b.y + b.height) - y).max(0.0),
        }
    }
    fn touches(self, b: Self) -> bool {
        self.x <= b.x + b.width
            && b.x <= self.x + self.width
            && self.y <= b.y + b.height
            && b.y <= self.y + self.height
    }
    fn js(self, vm: &mut Vm) -> std::result::Result<JsValue, JsError> {
        let value = vm.object()?;
        for (name, n) in [
            ("x", self.x),
            ("y", self.y),
            ("width", self.width),
            ("height", self.height),
            ("top", self.y),
            ("left", self.x),
            ("right", self.x + self.width),
            ("bottom", self.y + self.height),
        ] {
            vm.set(value.clone(), name, JsValue::Number(n))?;
        }
        Ok(value)
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GeometryBox {
    pub border: GeometryRect,
    pub content: GeometryRect,
    pub fixed: bool,
    pub clip: Option<GeometryRect>,
}
#[derive(Clone, Default)]
pub(crate) struct Watch {
    child: bool,
    attributes: bool,
    character: bool,
    subtree: bool,
    old_attribute: bool,
    old_character: bool,
    filter: Option<Vec<String>>,
    last: Option<Sample>,
}
#[derive(Clone, PartialEq)]
enum Sample {
    Resize([f64; 4]),
    Intersection(bool, usize),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Mutation,
    Resize,
    Intersection,
}
#[derive(Clone)]
enum Record {
    Mutation(Mutation),
    Resize {
        node: NodeId,
        geometry: GeometryBox,
    },
    Intersection {
        node: NodeId,
        bounds: GeometryRect,
        root: GeometryRect,
        intersection: GeometryRect,
        ratio: f64,
        intersecting: bool,
        time: f64,
    },
}
#[derive(Clone)]
pub(crate) struct Observer {
    pub callback: JsValue,
    pub targets: HashMap<NodeId, Watch>,
    mode: Mode,
    records: Vec<Record>,
    queued: bool,
    root: Option<NodeId>,
    margins: [(f64, bool); 4],
    thresholds: Vec<f64>,
}
impl Observer {
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.targets
            .keys()
            .copied()
            .chain(self.root)
            .chain(self.records.iter().flat_map(|record| match record {
                Record::Mutation(m) => m.nodes().collect::<Vec<_>>(),
                Record::Resize { node, .. } | Record::Intersection { node, .. } => vec![*node],
            }))
    }
    pub fn retained_bytes(&self) -> usize {
        self.targets.len() * 192
            + self
                .records
                .iter()
                .map(|r| match r {
                    Record::Mutation(m) => m.retained_bytes(),
                    _ => 256,
                })
                .sum::<usize>()
    }
}
impl WebHost {
    pub(super) fn observer_get(
        &self,
        vm: &mut Vm,
        id: u64,
        key: &str,
    ) -> std::result::Result<Option<JsValue>, JsError> {
        let s = self.state.borrow();
        let Some(Object {
            kind: Kind::Observer(observer),
            ..
        }) = s.objects.get(&id)
        else {
            return Ok(None);
        };
        Ok(match key {
            "observe" => Some(s.function("observer:observe")),
            "unobserve" if observer.mode != Mode::Mutation => {
                Some(s.function("observer:unobserve"))
            }
            "disconnect" => Some(s.function("observer:disconnect")),
            "takeRecords" if observer.mode != Mode::Resize => {
                Some(s.function("observer:takeRecords"))
            }
            "root" if observer.mode == Mode::Intersection => {
                Some(if let Some(node) = observer.root {
                    wrap(vm, s.dom.node_host(node), "Element")?
                } else {
                    JsValue::Null
                })
            }
            "thresholds" if observer.mode == Mode::Intersection => Some(
                vm.array(
                    observer
                        .thresholds
                        .iter()
                        .map(|n| JsValue::Number(*n))
                        .collect(),
                )?,
            ),
            "rootMargin" if observer.mode == Mode::Intersection => Some(JsValue::string(
                observer
                    .margins
                    .iter()
                    .map(|(n, percent)| format!("{n}{}", if *percent { "%" } else { "px" }))
                    .collect::<Vec<_>>()
                    .join(" "),
            )),
            _ => None,
        })
    }
    pub(super) fn observer_call(
        &self,
        vm: &mut Vm,
        name: &str,
        id: u64,
        args: &[JsValue],
    ) -> std::result::Result<Option<JsValue>, JsError> {
        if matches!(
            name,
            "MutationObserver" | "ResizeObserver" | "IntersectionObserver"
        ) {
            let callback = arg(args, 0);
            if !vm.callable(&callback) {
                return Err(error(vm, ApiError::new("TypeError", "observer callback")));
            }
            let mode = match name {
                "MutationObserver" => Mode::Mutation,
                "ResizeObserver" => Mode::Resize,
                _ => Mode::Intersection,
            };
            let mut root = None;
            let mut margins = [(0.0, false); 4];
            let mut thresholds = vec![0.0];
            if mode == Mode::Intersection {
                let value = get(vm, arg(args, 1), "root")?;
                if !matches!(value, JsValue::Undefined | JsValue::Null) {
                    root = Some(
                        vm.host_id(&value)
                            .and_then(|id| self.state.borrow().dom.node_id(id))
                            .ok_or_else(|| {
                                error(vm, ApiError::new("TypeError", "intersection root"))
                            })?,
                    );
                }
                let value = get(vm, arg(args, 1), "rootMargin")?;
                if value != JsValue::Undefined {
                    let value = vm.to_string(value)?;
                    margins = parse_margins(&value).map_err(|e| error(vm, e))?;
                }
                let value = get(vm, arg(args, 1), "threshold")?;
                if value != JsValue::Undefined {
                    let values = if vm.is_array(&value) {
                        vm.array_values(value)?
                    } else {
                        vec![value]
                    };
                    if values.len() > 128 {
                        return Err(error(
                            vm,
                            ApiError::new("QuotaExceededError", "intersection thresholds"),
                        ));
                    }
                    thresholds.clear();
                    for value in values {
                        let n = vm.to_number(value)?;
                        if !(0.0..=1.0).contains(&n) {
                            return Err(error(
                                vm,
                                ApiError::new("RangeError", "intersection threshold"),
                            ));
                        }
                        thresholds.push(n);
                    }
                    if thresholds.is_empty() {
                        thresholds.push(0.0);
                    }
                    thresholds.sort_by(f64::total_cmp);
                    thresholds.dedup();
                }
            }
            if self
                .state
                .borrow()
                .objects
                .values()
                .filter(|o| matches!(o.kind, Kind::Observer(_)))
                .count()
                >= 256
            {
                return Err(error(
                    vm,
                    ApiError::new("QuotaExceededError", "observer count"),
                ));
            }
            return self
                .allocate(
                    vm,
                    Kind::Observer(Observer {
                        callback,
                        targets: HashMap::new(),
                        mode,
                        records: Vec::new(),
                        queued: false,
                        root,
                        margins,
                        thresholds,
                    }),
                    name,
                )
                .map(Some);
        }
        if !name.starts_with("observer:") {
            return Ok(None);
        }
        if name == "observer:takeRecords" {
            self.collect_mutations(vm)?;
            let records = {
                let mut s = self.state.borrow_mut();
                let Some(Object {
                    kind: Kind::Observer(observer),
                    ..
                }) = s.objects.get_mut(&id)
                else {
                    return Err(error(vm, ApiError::new("TypeError", "observer receiver")));
                };
                std::mem::take(&mut observer.records)
            };
            return records_js(vm, &self.state.borrow().dom.clone(), records).map(Some);
        }
        let target = if matches!(name, "observer:observe" | "observer:unobserve") {
            Some(
                vm.host_id(&arg(args, 0))
                    .and_then(|id| self.state.borrow().dom.node_id(id))
                    .ok_or_else(|| error(vm, ApiError::new("TypeError", "observer target")))?,
            )
        } else {
            None
        };
        let mode = {
            let s = self.state.borrow();
            let Some(Object {
                kind: Kind::Observer(observer),
                ..
            }) = s.objects.get(&id)
            else {
                return Err(error(vm, ApiError::new("TypeError", "observer receiver")));
            };
            observer.mode
        };
        let watch = if name == "observer:observe" && mode == Mode::Mutation {
            let options = arg(args, 1);
            let attributes = get(vm, options.clone(), "attributes")?;
            let character = get(vm, options.clone(), "characterData")?;
            let old_attribute = get(vm, options.clone(), "attributeOldValue")?.truthy();
            let old_character = get(vm, options.clone(), "characterDataOldValue")?.truthy();
            let filter = get(vm, options.clone(), "attributeFilter")?;
            let filter = if filter != JsValue::Undefined {
                let mut names = Vec::new();
                for value in vm.array_values(filter)? {
                    let name = vm.to_string(value)?;
                    if name.len() > 1024 {
                        return Err(error(
                            vm,
                            ApiError::new("QuotaExceededError", "attribute filter name"),
                        ));
                    }
                    names.push(name);
                }
                if names.len() > 256 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "attribute filter"),
                    ));
                }
                Some(names)
            } else {
                None
            };
            let attributes = if attributes == JsValue::Undefined {
                old_attribute || filter.is_some()
            } else {
                attributes.truthy()
            };
            let character = if character == JsValue::Undefined {
                old_character
            } else {
                character.truthy()
            };
            let child = get(vm, options.clone(), "childList")?.truthy();
            if !(child || attributes || character)
                || !attributes && (old_attribute || filter.is_some())
                || !character && old_character
            {
                return Err(error(
                    vm,
                    ApiError::new("TypeError", "mutation observation options"),
                ));
            }
            Watch {
                child,
                attributes,
                character,
                old_attribute,
                old_character,
                filter,
                subtree: get(vm, options, "subtree")?.truthy(),
                last: None,
            }
        } else {
            Watch::default()
        };
        let mut s = self.state.borrow_mut();
        let total = s
            .objects
            .values()
            .filter_map(|o| match &o.kind {
                Kind::Observer(o) => Some(o.targets.len()),
                _ => None,
            })
            .sum::<usize>();
        let Some(Object {
            kind: Kind::Observer(observer),
            ..
        }) = s.objects.get_mut(&id)
        else {
            return Err(error(vm, ApiError::new("TypeError", "observer receiver")));
        };
        match name {
            "observer:observe" => {
                if total >= 32768 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "page observer targets"),
                    ));
                }
                if observer.targets.len() >= 1024
                    && !observer.targets.contains_key(&target.unwrap_or(0))
                {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "observer targets"),
                    ));
                }
                observer.targets.insert(target.unwrap_or(0), watch);
            }
            "observer:unobserve" => {
                observer.targets.remove(&target.unwrap_or(0));
            }
            "observer:disconnect" => {
                observer.targets.clear();
                observer.records.clear();
            }
            _ => {}
        }
        let observing=s.objects.values().any(|o|matches!(&o.kind,Kind::Observer(observer)if observer.mode==Mode::Mutation&&!observer.targets.is_empty()));
        s.dom.observe_mutations(observing);
        drop(s);
        if mode != Mode::Mutation {
            self.collect_geometry(vm)?;
        }
        Ok(Some(JsValue::Undefined))
    }
    pub(super) fn collect_mutations(&self, vm: &mut Vm) -> std::result::Result<(), JsError> {
        let handle = self.state.borrow().dom.clone();
        let mutations = handle
            .take_mutations()
            .map_err(|e| JsError::new(ErrorKind::MemoryLimit, e.to_string()))?;
        if mutations.is_empty() {
            return Ok(());
        }
        let mut s = self.state.borrow_mut();
        let mut budget = s
            .objects
            .values()
            .filter_map(|o| match &o.kind {
                Kind::Observer(o) => Some(o.retained_bytes()),
                _ => None,
            })
            .sum::<usize>();
        for object in s.objects.values_mut() {
            let Kind::Observer(observer) = &mut object.kind else {
                continue;
            };
            if observer.mode != Mode::Mutation {
                continue;
            }
            let mut transient = HashMap::<NodeId, Vec<NodeId>>::new();
            for mutation in &mutations {
                vm.checkpoint()?;
                let mut selected = false;
                let mut keep_old = false;
                for (&root, watch) in &observer.targets {
                    let inside = root == mutation.target
                        || watch.subtree
                            && (mutation.ancestors.contains(&root)
                                || transient.get(&root).is_some_and(|nodes| {
                                    nodes.iter().any(|&removed| {
                                        handle
                                            .with_document(|d| d.contains(removed, mutation.target))
                                    })
                                }));
                    let interested = match mutation.kind {
                        MutationKind::ChildList => watch.child,
                        MutationKind::Attributes => {
                            watch.attributes
                                && watch.filter.as_ref().is_none_or(|names| {
                                    mutation.name.as_ref().is_some_and(|n| names.contains(n))
                                })
                        }
                        MutationKind::CharacterData => watch.character,
                    };
                    if inside && watch.subtree && mutation.kind == MutationKind::ChildList {
                        transient
                            .entry(root)
                            .or_default()
                            .extend(mutation.removed.iter().copied());
                    }
                    if inside && interested {
                        selected = true;
                        keep_old |= match mutation.kind {
                            MutationKind::Attributes => watch.old_attribute,
                            MutationKind::CharacterData => watch.old_character,
                            _ => false,
                        };
                    }
                }
                if selected {
                    if observer.records.len() >= 1024 {
                        return Err(JsError::new(
                            ErrorKind::MemoryLimit,
                            "observer record quota",
                        ));
                    }
                    let mut record = mutation.clone();
                    if !keep_old {
                        record.old_value = None;
                    }
                    budget = budget.saturating_add(record.retained_bytes());
                    if budget > 32 * 1024 * 1024 {
                        return Err(JsError::new(
                            ErrorKind::MemoryLimit,
                            "page observer record quota",
                        ));
                    }
                    observer.records.push(Record::Mutation(record));
                }
            }
        }
        let mut queue = Vec::new();
        for (&id, object) in &mut s.objects {
            if let Kind::Observer(observer) = &mut object.kind {
                if observer.mode == Mode::Mutation
                    && !observer.records.is_empty()
                    && !observer.queued
                {
                    observer.queued = true;
                    queue.push(id);
                }
            }
        }
        drop(s);
        for id in queue {
            vm.queue_host_microtask(id)?;
        }
        Ok(())
    }
    pub(super) fn observer_microtask(
        &self,
        vm: &mut Vm,
        id: u64,
    ) -> std::result::Result<HostResult, JsError> {
        let (callback, records, handle) = {
            let mut s = self.state.borrow_mut();
            let handle = s.dom.clone();
            let Some(Object {
                kind: Kind::Observer(observer),
                ..
            }) = s.objects.get_mut(&id)
            else {
                return Ok(HostResult::value(JsValue::Undefined));
            };
            observer.queued = false;
            (
                observer.callback.clone(),
                std::mem::take(&mut observer.records),
                handle,
            )
        };
        if records.is_empty() {
            return Ok(HostResult::value(JsValue::Undefined));
        }
        let records = records_js(vm, &handle, records)?;
        let observer = wrap(vm, id, "Observer")?;
        let mut result = HostResult::value(JsValue::Undefined);
        result
            .calls
            .push((callback, observer.clone(), vec![records, observer]));
        result.catch_callback_errors = true;
        Ok(result)
    }
    fn collect_geometry(&self, vm: &mut Vm) -> std::result::Result<(), JsError> {
        let mut s = self.state.borrow_mut();
        if !s.geometry_ready {
            return Ok(());
        }
        let geometry = s.geometry.clone();
        let viewport = s.viewport;
        let handle = s.dom.clone();
        for object in s.objects.values_mut() {
            let Kind::Observer(observer) = &mut object.kind else {
                continue;
            };
            if observer.mode == Mode::Mutation {
                continue;
            }
            let base = observer
                .root
                .and_then(|id| geometry.get(&id).map(|g| g.border))
                .unwrap_or(viewport);
            let margins = observer
                .margins
                .map(|(n, p)| if p { n * base.width / 100.0 } else { n });
            let root = GeometryRect {
                x: base.x - margins[3],
                y: base.y - margins[0],
                width: (base.width + margins[1] + margins[3]).max(0.0),
                height: (base.height + margins[0] + margins[2]).max(0.0),
            };
            for (&node, watch) in &mut observer.targets {
                vm.checkpoint()?;
                let present =
                    geometry.contains_key(&node) && handle.with_document(|d| d.contains(0, node));
                let mut bounds = geometry
                    .get(&node)
                    .copied()
                    .filter(|_| handle.with_document(|d| d.contains(0, node)))
                    .unwrap_or_default();
                if bounds.fixed {
                    bounds.border.x += viewport.x;
                    bounds.border.y += viewport.y;
                    bounds.content.x += viewport.x;
                    bounds.content.y += viewport.y;
                    if let Some(clip) = &mut bounds.clip {
                        clip.x += viewport.x;
                        clip.y += viewport.y;
                    }
                }
                let (sample, record) = if observer.mode == Mode::Resize {
                    (
                        Sample::Resize([
                            bounds.content.width,
                            bounds.content.height,
                            bounds.border.width,
                            bounds.border.height,
                        ]),
                        Record::Resize {
                            node,
                            geometry: bounds,
                        },
                    )
                } else {
                    let inside_root = observer
                        .root
                        .is_none_or(|root| handle.with_document(|d| d.contains(root, node)));
                    let visible = bounds
                        .clip
                        .map_or(bounds.border, |clip| bounds.border.intersection(clip));
                    let intersecting = present
                        && inside_root
                        && bounds.border.touches(root)
                        && bounds
                            .clip
                            .is_none_or(|clip| clip.touches(bounds.border) && clip.touches(root));
                    let intersection = if intersecting {
                        visible.intersection(root)
                    } else {
                        GeometryRect::default()
                    };
                    let area = bounds.border.width * bounds.border.height;
                    let ratio = if area > 0.0 {
                        intersection.width * intersection.height / area
                    } else if intersecting {
                        1.0
                    } else {
                        0.0
                    };
                    let threshold = observer
                        .thresholds
                        .iter()
                        .take_while(|n| ratio >= **n)
                        .count();
                    (
                        Sample::Intersection(intersecting, threshold),
                        Record::Intersection {
                            node,
                            bounds: bounds.border.relative(viewport),
                            root: root.relative(viewport),
                            intersection: intersection.relative(viewport),
                            ratio,
                            intersecting,
                            time: handle.timestamp(),
                        },
                    )
                };
                if watch.last.as_ref() != Some(&sample) {
                    watch.last = Some(sample);
                    if observer.records.len() >= 1024 {
                        return Err(JsError::new(
                            ErrorKind::MemoryLimit,
                            "observer record quota",
                        ));
                    }
                    observer.records.push(record);
                }
            }
        }
        Ok(())
    }
}
impl PageRuntime {
    pub fn geometry(
        &mut self,
        boxes: Vec<(NodeId, GeometryBox)>,
        viewport: GeometryRect,
    ) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        if boxes.len() > 500_000
            || !viewport.valid()
            || boxes
                .iter()
                .any(|(_, b)| !b.border.valid() || !b.content.valid())
        {
            return self.dom.stop(JsError::new(
                ErrorKind::MemoryLimit,
                "geometry snapshot bounds",
            ));
        }
        {
            let mut s = self.state.borrow_mut();
            s.geometry = boxes.into_iter().collect();
            s.geometry_ready = true;
            s.viewport = viewport;
        }
        WebHost {
            state: self.state.clone(),
        }
        .collect_geometry(&mut self.dom.vm)?;
        self.drain_jobs()
    }
    pub fn viewport(&mut self, viewport: GeometryRect) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        if !viewport.valid() {
            return Ok(());
        }
        self.state.borrow_mut().viewport = viewport;
        WebHost {
            state: self.state.clone(),
        }
        .collect_geometry(&mut self.dom.vm)?;
        self.drain_jobs()
    }
    pub(super) fn observer_checkpoint(&mut self) -> std::result::Result<bool, JsError> {
        WebHost {
            state: self.state.clone(),
        }
        .collect_mutations(&mut self.dom.vm)?;
        let mut s = self.state.borrow_mut();
        let mut ids = Vec::new();
        for (&id, object) in &mut s.objects {
            if let Kind::Observer(observer) = &mut object.kind {
                if !observer.records.is_empty() && !observer.queued {
                    observer.queued = true;
                    ids.push(id);
                }
            }
        }
        for id in &ids {
            s.jobs.push_back(Job::Observer { id: *id });
        }
        Ok(!ids.is_empty())
    }
    pub(super) fn observer_callback(&mut self, id: u64) -> std::result::Result<(), JsError> {
        let (callback, records, handle) = {
            let mut s = self.state.borrow_mut();
            let handle = s.dom.clone();
            let Some(Object {
                kind: Kind::Observer(observer),
                ..
            }) = s.objects.get_mut(&id)
            else {
                return Ok(());
            };
            observer.queued = false;
            (
                observer.callback.clone(),
                std::mem::take(&mut observer.records),
                handle,
            )
        };
        if records.is_empty() {
            return Ok(());
        }
        let records = records_js(&mut self.dom.vm, &handle, records)?;
        let observer = wrap(&mut self.dom.vm, id, "Observer")?;
        if let Err(error) =
            self.dom
                .call_in_task(callback, observer.clone(), vec![records, observer])
        {
            if error.is_resource_limit() {
                return Err(error);
            }
            handle.console(error.to_string());
        }
        Ok(())
    }
}
fn parse_margins(input: &str) -> Result<[(f64, bool); 4]> {
    let mut values = Vec::new();
    for value in input.split_ascii_whitespace() {
        if values.len() >= 4 {
            return Err(ApiError::new("SyntaxError", "rootMargin components"));
        }
        let (n, p) = if let Some(n) = value.strip_suffix("px") {
            (n, false)
        } else if let Some(n) = value.strip_suffix('%') {
            (n, true)
        } else {
            return Err(ApiError::new("SyntaxError", "rootMargin units"));
        };
        let n = n
            .parse::<f64>()
            .map_err(|_| ApiError::new("SyntaxError", "rootMargin"))?;
        if !n.is_finite() || n.abs() > 1_000_000.0 {
            return Err(ApiError::new("SyntaxError", "rootMargin bounds"));
        }
        values.push((n, p));
    }
    Ok(match values.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Err(ApiError::new("SyntaxError", "rootMargin components")),
    })
}
fn records_js(
    vm: &mut Vm,
    handle: &DomHandle,
    records: Vec<Record>,
) -> std::result::Result<JsValue, JsError> {
    let node = |vm: &mut Vm, id: Option<NodeId>| {
        id.map(|id| wrap(vm, handle.node_host(id), "Node"))
            .transpose()
            .map(|v| v.unwrap_or(JsValue::Null))
    };
    let mut out = Vec::new();
    for record in records {
        vm.checkpoint()?;
        let value = vm.object()?;
        match record {
            Record::Mutation(record) => {
                let target = node(vm, Some(record.target))?;
                vm.set(value.clone(), "target", target)?;
                vm.set(
                    value.clone(),
                    "type",
                    JsValue::string(match record.kind {
                        MutationKind::ChildList => "childList",
                        MutationKind::Attributes => "attributes",
                        MutationKind::CharacterData => "characterData",
                    }),
                )?;
                for (name, nodes) in [
                    ("addedNodes", record.added),
                    ("removedNodes", record.removed),
                ] {
                    let mut list = Vec::new();
                    for id in nodes {
                        list.push(node(vm, Some(id))?);
                    }
                    let list = vm.array(list)?;
                    vm.set(value.clone(), name, list)?;
                }
                for (name, id) in [
                    ("previousSibling", record.previous),
                    ("nextSibling", record.next),
                ] {
                    let n = node(vm, id)?;
                    vm.set(value.clone(), name, n)?;
                }
                vm.set(
                    value.clone(),
                    "attributeName",
                    record.name.map(JsValue::string).unwrap_or(JsValue::Null),
                )?;
                vm.set(value.clone(), "attributeNamespace", JsValue::Null)?;
                vm.set(
                    value.clone(),
                    "oldValue",
                    record
                        .old_value
                        .map(JsValue::string)
                        .unwrap_or(JsValue::Null),
                )?;
            }
            Record::Resize { node: id, geometry } => {
                let target = node(vm, Some(id))?;
                vm.set(value.clone(), "target", target)?;
                let content = GeometryRect {
                    x: 0.0,
                    y: 0.0,
                    ..geometry.content
                }
                .js(vm)?;
                vm.set(value.clone(), "contentRect", content)?;
                for (name, rect) in [
                    ("contentBoxSize", geometry.content),
                    ("borderBoxSize", geometry.border),
                ] {
                    let size = vm.object()?;
                    vm.set(size.clone(), "inlineSize", JsValue::Number(rect.width))?;
                    vm.set(size.clone(), "blockSize", JsValue::Number(rect.height))?;
                    let list = vm.array(vec![size])?;
                    vm.set(value.clone(), name, list)?;
                }
            }
            Record::Intersection {
                node: id,
                bounds,
                root,
                intersection,
                ratio,
                intersecting,
                time,
            } => {
                let target = node(vm, Some(id))?;
                vm.set(value.clone(), "target", target)?;
                for (name, rect) in [
                    ("boundingClientRect", bounds),
                    ("rootBounds", root),
                    ("intersectionRect", intersection),
                ] {
                    let rect = rect.js(vm)?;
                    vm.set(value.clone(), name, rect)?;
                }
                vm.set(value.clone(), "intersectionRatio", JsValue::Number(ratio))?;
                vm.set(value.clone(), "isIntersecting", JsValue::Bool(intersecting))?;
                vm.set(value.clone(), "time", JsValue::Number(time))?;
            }
        }
        out.push(value);
    }
    vm.array(out)
}
