//! Geometry and measurement reuse with identity/constraint/dependency checks.
use super::*;
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Key {
    element: ElementId,
    role: u8,
    occurrence: usize,
}
#[derive(Clone, PartialEq)]
struct Snapshot {
    kind: RenderKind,
    style: Arc<ComputedStyle>,
    span: [usize; 2],
    children: Vec<Key>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Signature {
    width: u32,
    height: Option<u32>,
    forced_width: Option<u32>,
    forced_height: Option<u32>,
    atomic: bool,
    item: bool,
}
#[derive(Clone)]
pub(super) struct Bundle {
    signature: Signature,
    origin: (f32, f32),
    boxes: Vec<LayoutBox>,
    hits: Vec<HitRegion>,
    targets: Vec<TargetRegion>,
    lines: usize,
    glyphs: usize,
}
#[derive(Default)]
pub struct LayoutCache {
    previous: HashMap<Key, Snapshot>,
    pub(super) keys: Vec<Key>,
    safe: Vec<bool>,
    entries: HashMap<Key, Bundle>,
    measurements: HashMap<(Key, Signature), (Size, f32)>,
    margins: HashMap<(Key, u32, Option<u32>, bool), Margins>,
    images: HashMap<Url, u64>,
    viewport: Option<Size>,
    last: Option<LayoutResult>,
    bytes: usize,
}
impl LayoutCache {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn layout(
        &mut self,
        viewport: Size,
        tree: &mut RenderTree,
        opts: LayoutOpts<'_>,
    ) -> LayoutResult {
        if self.viewport != Some(viewport) {
            self.clear();
            self.viewport = Some(viewport);
        }
        let changed_urls = opts
            .images
            .iter()
            .filter(|(url, image)| self.images.get(*url) != Some(&image.id))
            .map(|(u, _)| u.clone())
            .chain(
                self.images
                    .keys()
                    .filter(|u| !opts.images.contains_key(*u))
                    .cloned(),
            )
            .collect::<HashSet<_>>();
        self.images = opts.images.iter().map(|(u, i)| (u.clone(), i.id)).collect();
        let mut counts = HashMap::new();
        self.keys.clear();
        self.safe.clear();
        for n in &tree.nodes {
            let role = match n.kind {
                RenderKind::Block => 0,
                RenderKind::Inline => 1,
                RenderKind::InlineBlock => 2,
                RenderKind::AnonymousBlock => 3,
                RenderKind::Text(_) => 4,
                RenderKind::Break => 5,
                RenderKind::Replaced(_) => 6,
            };
            let occurrence = counts.entry((n.element, role)).or_insert(0);
            self.keys.push(Key {
                element: n.element,
                role,
                occurrence: *occurrence,
            });
            *occurrence += 1;
        }
        let mut current = HashMap::new();
        for i in 0..tree.nodes.len() {
            let n = &tree.nodes[i];
            let snapshot = Snapshot {
                kind: n.kind.clone(),
                style: n.style.clone(),
                span: n.span,
                children: n.children.iter().map(|&c| self.keys[c]).collect(),
            };
            let resource = matches!(&n.kind,RenderKind::Replaced(Replaced::Image{url:Some(u),..}) if changed_urls.contains(u))
                || matches!(&n.style.background_image,Some(BackgroundImage::Url(u)) if changed_urls.contains(u));
            let dirty = self.previous.get(&self.keys[i]) != Some(&snapshot)
                || resource
                || n.children.iter().any(|&c| tree.nodes[c].dirty);
            self.safe.push(
                !n.out_of_flow() && !n.floating() && n.children.iter().all(|&c| self.safe[c]),
            );
            tree.nodes[i].dirty = dirty;
            current.insert(self.keys[i], snapshot);
            if dirty {
                self.entries.remove(&self.keys[i]);
                self.measurements.retain(|(key, _), _| *key != self.keys[i]);
                self.margins
                    .retain(|(key, _, _, _), _| *key != self.keys[i]);
            }
        }
        self.previous = current;
        self.entries.retain(|k, _| self.previous.contains_key(k));
        self.bytes = self.entries.values().map(Self::bundle_bytes).sum();
        let dirty = tree.nodes.iter().filter(|n| n.dirty).count();
        if dirty == 0 {
            if let Some(last) = &self.last {
                let mut result = last.clone();
                result.stats.reused_boxes = result.boxes.len();
                result.stats.reused_subtrees = 1;
                result.stats.dirty_nodes = 0;
                return result;
            }
        }
        let mut result = super::perform(viewport, tree, opts, Some(self));
        result.stats.dirty_nodes = dirty;
        self.last = Some(result.clone());
        result
    }
    fn bundle_bytes(b: &Bundle) -> usize {
        b.boxes.len() * 256
            + b.hits.len() * 128
            + b.targets.len() * 64
            + b.glyphs * std::mem::size_of::<Glyph>()
            + b.boxes
                .iter()
                .flat_map(|b| &b.content)
                .map(|c| match c {
                    Content::Text(t) => t.text.len(),
                    _ => 0,
                })
                .sum::<usize>()
    }
}
impl Engine<'_> {
    fn signature(
        &self,
        index: usize,
        width: f32,
        height: Option<f32>,
        ancestors: &[usize],
        atomic: bool,
    ) -> Signature {
        Signature {
            width: width.to_bits(),
            height: height.map(f32::to_bits),
            forced_width: self.used[index].width.map(f32::to_bits),
            forced_height: self.used[index].height.map(f32::to_bits),
            atomic,
            item: ancestors.last().is_some_and(|&i| {
                matches!(
                    self.result.boxes[i].style.layout.mode,
                    Mode::Flex | Mode::Grid
                )
            }),
        }
    }
    pub(super) fn cached_measure(
        &self,
        index: usize,
        width: f32,
        height: Option<f32>,
        used: UsedSize,
    ) -> Option<(Size, f32)> {
        if self.tree.nodes[index].dirty {
            return None;
        }
        let cache = self.cache.as_ref()?;
        let sig = Signature {
            width: width.to_bits(),
            height: height.map(f32::to_bits),
            forced_width: used.width.map(f32::to_bits),
            forced_height: used.height.map(f32::to_bits),
            atomic: false,
            item: false,
        };
        cache.measurements.get(&(cache.keys[index], sig)).copied()
    }
    pub(super) fn save_measure(
        &mut self,
        index: usize,
        width: f32,
        height: Option<f32>,
        used: UsedSize,
        result: (Size, f32),
    ) {
        if let Some(cache) = &mut self.cache {
            if cache.measurements.len() < 8192 {
                let sig = Signature {
                    width: width.to_bits(),
                    height: height.map(f32::to_bits),
                    forced_width: used.width.map(f32::to_bits),
                    forced_height: used.height.map(f32::to_bits),
                    atomic: false,
                    item: false,
                };
                cache.measurements.insert((cache.keys[index], sig), result);
            }
        }
    }
    pub(super) fn margin_cached(&self, index: usize, width: f32, root: bool) -> Option<Margins> {
        if self.tree.nodes[index].dirty {
            return None;
        }
        let c = self.cache.as_ref()?;
        c.margins
            .get(&(
                c.keys[index],
                width.to_bits(),
                self.used[index].width.map(f32::to_bits),
                root,
            ))
            .copied()
    }
    pub(super) fn save_margin(&mut self, index: usize, width: f32, root: bool, m: Margins) {
        if let Some(c) = &mut self.cache {
            if c.margins.len() < 65536 {
                c.margins.insert(
                    (
                        c.keys[index],
                        width.to_bits(),
                        self.used[index].width.map(f32::to_bits),
                        root,
                    ),
                    m,
                );
            }
        }
    }
    pub(super) fn cached_block(
        &mut self,
        index: usize,
        rect: Rect,
        top: f32,
        height: Option<f32>,
        ancestors: &[usize],
        atomic: bool,
    ) -> Option<usize> {
        if self.measuring
            || self.tree.nodes[index].dirty
            || ancestors.last().is_some_and(|&p| {
                self.float_regions
                    .get(&self.result.boxes[p].formatting_context)
                    .is_some_and(|r| !r.is_empty())
            })
        {
            return None;
        }
        let c = self.cache.as_ref()?;
        if !c.safe[index] {
            return None;
        }
        let key = c.keys[index];
        let signature = self.signature(index, rect.width, height, ancestors, atomic);
        let bundle = c
            .entries
            .get(&key)
            .filter(|b| b.signature == signature)?
            .clone();
        let start = self.result.boxes.len();
        let dx = rect.x - bundle.origin.0;
        let dy = top - bundle.origin.1;
        let new_context = ancestors
            .last()
            .map_or(start, |&p| self.result.boxes[p].formatting_context);
        for (offset, mut b) in bundle.boxes.into_iter().enumerate() {
            b.parent = if offset == 0 {
                ancestors.last().copied()
            } else {
                b.parent.map(|p| p + start)
            };
            b.formatting_context = if b.formatting_context == usize::MAX {
                new_context
            } else {
                b.formatting_context + start
            };
            for c in &mut b.content {
                if let Content::Box(i) = c {
                    *i += start;
                }
            }
            self.result.boxes.push(b);
        }
        for mut h in bundle.hits {
            let mut path = ancestors.to_vec();
            path.extend(h.ancestors.iter().map(|i| i + start));
            h.ancestors = path;
            self.result.hits.push(h);
        }
        for mut h in bundle.targets {
            let mut path = ancestors.to_vec();
            path.extend(h.ancestors.iter().map(|i| i + start));
            h.ancestors = path;
            self.result.targets.push(h);
        }
        self.shift_box(start, dx, dy);
        self.result.stats.lines += bundle.lines;
        self.result.stats.glyphs += bundle.glyphs;
        self.result.stats.reused_boxes += self.result.boxes.len() - start;
        self.result.stats.reused_subtrees += 1;
        Some(start)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn save_block(
        &mut self,
        index: usize,
        rect: Rect,
        top: f32,
        height: Option<f32>,
        ancestors: &[usize],
        atomic: bool,
        start: usize,
        lines: usize,
        glyphs: usize,
    ) {
        if self.measuring || self.result.stats.truncated || self.result.boxes.len() - start > 64 {
            return;
        }
        let signature = self.signature(index, rect.width, height, ancestors, atomic);
        let Some(cache) = &mut self.cache else {
            return;
        };
        if !cache.safe[index] {
            return;
        }
        let mut boxes = self.result.boxes[start..].to_vec();
        for b in &mut boxes {
            b.parent = b.parent.filter(|&p| p >= start).map(|p| p - start);
            b.formatting_context = if b.formatting_context >= start {
                b.formatting_context - start
            } else {
                usize::MAX
            };
            for c in &mut b.content {
                if let Content::Box(i) = c {
                    *i -= start;
                }
            }
        }
        let hits = self
            .result
            .hits
            .iter()
            .filter(|h| h.ancestors.contains(&start))
            .cloned()
            .map(|mut h| {
                h.ancestors = h
                    .ancestors
                    .into_iter()
                    .filter(|&i| i >= start)
                    .map(|i| i - start)
                    .collect();
                h
            })
            .collect();
        let targets = self
            .result
            .targets
            .iter()
            .filter(|h| h.ancestors.contains(&start))
            .cloned()
            .map(|mut h| {
                h.ancestors = h
                    .ancestors
                    .into_iter()
                    .filter(|&i| i >= start)
                    .map(|i| i - start)
                    .collect();
                h
            })
            .collect();
        let bundle = Bundle {
            signature,
            origin: (rect.x, top),
            boxes,
            hits,
            targets,
            lines: self.result.stats.lines - lines,
            glyphs: self.result.stats.glyphs - glyphs,
        };
        let bytes = LayoutCache::bundle_bytes(&bundle);
        if cache.bytes + bytes <= 32 * 1024 * 1024 {
            if let Some(old) = cache.entries.insert(cache.keys[index], bundle) {
                cache.bytes = cache.bytes.saturating_sub(LayoutCache::bundle_bytes(&old));
            }
            cache.bytes += bytes;
        }
    }
}
