use super::*;
use crate::http::{Credentials, Mode};
const GLOBALS: &[&str] = &[
    "MutationObserver",
    "ResizeObserver",
    "IntersectionObserver",
    "fetch",
    "Headers",
    "Request",
    "Response",
    "FormData",
    "URLSearchParams",
    "URL",
    "Blob",
    "FileReader",
    "XMLHttpRequest",
    "Worker",
    "WebSocket",
    "EventSource",
    "alert",
    "confirm",
    "prompt",
    "open",
    "Notification",
    "DOMException",
];
const METHODS: &[&str] = &[
    "url:createObjectURL",
    "url:revokeObjectURL",
    "fullscreen:request",
    "fullscreen:exit",
    "canvas:getContext",
    "canvas:fillRect",
    "canvas:clearRect",
    "canvas:strokeRect",
    "canvas:beginPath",
    "canvas:closePath",
    "canvas:moveTo",
    "canvas:lineTo",
    "canvas:rect",
    "canvas:arc",
    "canvas:quadraticCurveTo",
    "canvas:bezierCurveTo",
    "canvas:fill",
    "canvas:stroke",
    "canvas:save",
    "canvas:restore",
    "canvas:translate",
    "canvas:scale",
    "canvas:rotate",
    "canvas:transform",
    "canvas:setTransform",
    "canvas:resetTransform",
    "canvas:drawImage",
    "canvas:fillText",
    "canvas:measureText",
    "canvas:getImageData",
    "canvas:putImageData",
    "observer:observe",
    "observer:unobserve",
    "observer:disconnect",
    "observer:takeRecords",
    "event:preventDefault",
    "db:listContains",
    "db:listItem",
    "db:open",
    "db:deleteDatabase",
    "db:createObjectStore",
    "db:deleteObjectStore",
    "db:transaction",
    "db:close",
    "db:objectStore",
    "db:abort",
    "db:commit",
    "db:createIndex",
    "db:deleteIndex",
    "db:index",
    "db:put",
    "db:add",
    "db:get",
    "db:getKey",
    "db:getAll",
    "db:delete",
    "db:clear",
    "db:count",
    "headers:append",
    "headers:set",
    "headers:get",
    "headers:has",
    "headers:delete",
    "headers:entries",
    "headers:keys",
    "headers:values",
    "body:text",
    "body:json",
    "body:blob",
    "body:clone",
    "storage:getItem",
    "storage:setItem",
    "storage:removeItem",
    "storage:clear",
    "storage:key",
    "location:assign",
    "location:replace",
    "location:reload",
    "location:toString",
    "history:pushState",
    "history:replaceState",
    "history:back",
    "history:forward",
    "history:go",
    "params:append",
    "params:set",
    "params:get",
    "params:getAll",
    "params:has",
    "params:delete",
    "params:sort",
    "params:toString",
    "params:entries",
    "url:toString",
    "form:append",
    "form:set",
    "form:get",
    "form:getAll",
    "form:has",
    "form:delete",
    "blob:text",
    "blob:slice",
    "reader:readAsText",
    "reader:readAsDataURL",
    "reader:abort",
    "xhr:open",
    "xhr:send",
    "xhr:abort",
    "xhr:setRequestHeader",
    "xhr:getResponseHeader",
    "xhr:getAllResponseHeaders",
    "xhr:overrideMimeType",
    "events:add",
    "events:remove",
    "worker:postMessage",
    "worker:terminate",
    "socket:send",
    "socket:close",
    "sse:close",
    "clipboard:readText",
    "clipboard:writeText",
    "geo:getCurrentPosition",
    "permissions:query",
    "storageManager:persist",
    "storageManager:estimate",
    "notification:requestPermission",
];
pub fn install(runtime: &mut PageRuntime) -> std::result::Result<(), JsError> {
    for name in GLOBALS.iter().chain(METHODS) {
        let value = runtime.dom.vm.native(&format!("host:web:{name}"))?;
        runtime
            .state
            .borrow_mut()
            .functions
            .insert((*name).into(), value.clone());
        if GLOBALS.contains(name) {
            runtime.dom.vm.set_global(name, value.clone())?;
            if name.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
                let prototype = runtime.dom.vm.object()?;
                runtime
                    .dom
                    .vm
                    .set(prototype.clone(), "constructor", value.clone())?;
                if let Some(id) = value.object() {
                    runtime.dom.vm.define(
                        id,
                        "prototype".into(),
                        dom_bindings::js::value::Property {
                            writable: false,
                            enumerable: false,
                            configurable: false,
                            ..dom_bindings::js::value::Property::data(prototype)
                        },
                    )?;
                }
            }
        }
        if *name == "XMLHttpRequest" {
            for (key, v) in [
                ("UNSENT", 0.0),
                ("OPENED", 1.0),
                ("HEADERS_RECEIVED", 2.0),
                ("LOADING", 3.0),
                ("DONE", 4.0),
            ] {
                runtime.dom.vm.set(value.clone(), key, JsValue::Number(v))?;
            }
        }
        if *name == "WebSocket" {
            for (key, v) in [
                ("CONNECTING", 0.0),
                ("OPEN", 1.0),
                ("CLOSING", 2.0),
                ("CLOSED", 3.0),
            ] {
                runtime.dom.vm.set(value.clone(), key, JsValue::Number(v))?;
            }
        }
        if *name == "Notification" {
            let method = runtime
                .state
                .borrow()
                .function("notification:requestPermission");
            if method != JsValue::Undefined {
                runtime.dom.vm.set(value, "requestPermission", method)?;
            }
        }
    }
    let notification = runtime.state.borrow().function("Notification");
    let url = runtime.state.borrow().function("URL");
    for (key, name) in [
        ("createObjectURL", "url:createObjectURL"),
        ("revokeObjectURL", "url:revokeObjectURL"),
    ] {
        let method = runtime.state.borrow().function(name);
        runtime.dom.vm.set(url.clone(), key, method)?;
    }
    let method = runtime
        .state
        .borrow()
        .function("notification:requestPermission");
    runtime
        .dom
        .vm
        .set(notification, "requestPermission", method)?;
    {
        let mut s = runtime.state.borrow_mut();
        for (id, kind) in [
            (LOCATION, Kind::Location),
            (HISTORY, Kind::History),
            (NAVIGATOR, Kind::Navigator),
            (LOCAL, Kind::Storage(false)),
            (SESSION, Kind::Storage(true)),
            (CLIPBOARD, Kind::Clipboard),
            (GEO, Kind::Geo),
            (PERMISSIONS, Kind::Permissions),
            (STORAGE_MANAGER, Kind::StorageManager),
            (INDEXED_DB, Kind::IndexedDb),
        ] {
            s.objects.insert(
                id,
                Object {
                    kind,
                    properties: BTreeMap::new(),
                    listeners: Vec::new(),
                },
            );
        }
    }
    runtime.dom.set_extension(Box::new(WebHost {
        state: runtime.state.clone(),
    }));
    for (name, id, class) in [
        ("location", LOCATION, "Location"),
        ("history", HISTORY, "History"),
        ("navigator", NAVIGATOR, "Navigator"),
        ("localStorage", LOCAL, "Storage"),
        ("sessionStorage", SESSION, "Storage"),
        ("indexedDB", INDEXED_DB, "IDBFactory"),
    ] {
        let value = wrap(&mut runtime.dom.vm, id, class)?;
        runtime.dom.vm.set_global(name, value)?;
    }
    let ctor = runtime.state.borrow().function("Notification");
    let permission = {
        let state = runtime.state.borrow();
        state
            .config
            .profile
            .permission(&state.dom.url(), "notifications")
            .unwrap_or_else(|| "default".into())
    };
    runtime
        .dom
        .vm
        .set(ctor, "permission", JsValue::string(permission))?;
    Ok(())
}
fn headers(
    vm: &mut Vm,
    value: JsValue,
    state: &Rc<RefCell<State>>,
) -> std::result::Result<Headers, JsError> {
    if let Some((h, _, _)) = vm.host_id(&value).and_then(|id| state.borrow().headers(id)) {
        return Ok(h);
    }
    let mut result = Headers::default();
    if value == JsValue::Undefined {
        return Ok(result);
    }
    if value.object().is_none() {
        return Err(error(
            vm,
            ApiError::new("TypeError", "Headers initializer must be an object"),
        ));
    }
    let length = get(vm, value.clone(), "length")?;
    if let JsValue::Number(n) = length {
        if !(0.0..=256.0).contains(&n) {
            return Err(error(vm, ApiError::new("TypeError", "header entries")));
        }
        for i in 0..n as usize {
            let entry = vm.get(value.clone(), &i.to_string())?;
            if get(vm, entry.clone(), "length")? != JsValue::Number(2.0) {
                return Err(error(
                    vm,
                    ApiError::new("TypeError", "header entry must contain two values"),
                ));
            }
            let name = vm.get(entry.clone(), "0")?;
            let val = vm.get(entry, "1")?;
            let name = vm.to_string(name)?;
            let val = vm.to_string(val)?;
            result.append(&name, &val).map_err(|e| error(vm, e))?;
        }
    } else {
        for key in vm.keys(&value, true)? {
            if let dom_bindings::js::PropertyKey::String(key) = key {
                let value = vm.get(value.clone(), &key)?;
                let value = vm.to_string(value)?;
                result.append(&key, &value).map_err(|e| error(vm, e))?;
            }
        }
    }
    Ok(result)
}
fn body(
    vm: &mut Vm,
    value: JsValue,
    state: &Rc<RefCell<State>>,
) -> std::result::Result<(Vec<u8>, Option<String>), JsError> {
    if matches!(value, JsValue::Undefined | JsValue::Null) {
        return Ok((Vec::new(), None));
    }
    if let Some(kind) = vm
        .host_id(&value)
        .and_then(|id| state.borrow().objects.get(&id).map(|o| o.kind.clone()))
    {
        match kind {
            Kind::Blob { bytes, mime } => {
                return Ok((bytes.clone(), (!mime.is_empty()).then(|| mime.clone())))
            }
            Kind::Params { values, .. } => {
                return Ok((
                    url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(&values)
                        .finish()
                        .into_bytes(),
                    Some("application/x-www-form-urlencoded;charset=UTF-8".into()),
                ))
            }
            Kind::FormData(values) => {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                let boundary = format!(
                    "----RustBrowserForm{}",
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                let mut bytes = Vec::new();
                for (name, value) in values {
                    let name = name.replace(['\r', '\n', '"'], "_");
                    bytes.extend_from_slice(
                        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"")
                            .as_bytes(),
                    );
                    match value {
                        FormValue::Text(text) => {
                            bytes.extend_from_slice(b"\r\n\r\n");
                            bytes.extend_from_slice(text.as_bytes());
                        }
                        FormValue::Blob {
                            bytes: part,
                            mime,
                            name,
                        } => {
                            let name = name.replace(['\r', '\n', '"'], "_");
                            bytes.extend_from_slice(
                                format!("; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n")
                                    .as_bytes(),
                            );
                            bytes.extend_from_slice(&part);
                        }
                    }
                    bytes.extend_from_slice(b"\r\n");
                    if bytes.len() > MAX_BODY {
                        return Err(error(vm, ApiError::new("QuotaExceededError", "form body")));
                    }
                }
                bytes.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                return Ok((
                    bytes,
                    Some(format!("multipart/form-data; boundary={boundary}")),
                ));
            }
            _ => {}
        }
    }
    let text = vm.to_string(value)?;
    Ok((text.into_bytes(), Some("text/plain;charset=UTF-8".into())))
}
fn request(
    vm: &mut Vm,
    input: JsValue,
    options: JsValue,
    state: &Rc<RefCell<State>>,
) -> std::result::Result<Request, JsError> {
    let mut result = if let Some(Kind::Request(r)) = vm
        .host_id(&input)
        .and_then(|id| state.borrow().objects.get(&id).map(|o| o.kind.clone()))
    {
        r.clone()
    } else {
        let text = vm.to_string(input)?;
        Request::get(web_url(&state.borrow().dom.url(), &text).map_err(|e| error(vm, e))?)
    };
    let method = get(vm, options.clone(), "method")?;
    if method != JsValue::Undefined {
        result.method = vm.to_string(method)?;
    }
    let h = get(vm, options.clone(), "headers")?;
    if h != JsValue::Undefined {
        result.headers = headers(vm, h, state)?;
    }
    let value = get(vm, options.clone(), "body")?;
    if value != JsValue::Undefined {
        let (bytes, mime) = body(vm, value, state)?;
        result.body = bytes;
        if result.headers.get("content-type").is_none() {
            if let Some(mime) = mime {
                result
                    .headers
                    .set("content-type", &mime)
                    .map_err(|e| error(vm, e))?;
            }
        }
    }
    for key in ["mode", "credentials", "redirect"] {
        let value = get(vm, options.clone(), key)?;
        if value == JsValue::Undefined {
            continue;
        }
        let value = vm.to_string(value)?;
        match (key, value.as_str()) {
            ("mode", "cors") => result.mode = Mode::Cors,
            ("mode", "same-origin") => result.mode = Mode::SameOrigin,
            ("mode", "no-cors") => result.mode = Mode::NoCors,
            ("credentials", "omit") => result.credentials = Credentials::Omit,
            ("credentials", "same-origin") => result.credentials = Credentials::SameOrigin,
            ("credentials", "include") => result.credentials = Credentials::Include,
            ("redirect", "follow" | "manual" | "error") => result.redirect = value,
            _ => {
                return Err(error(
                    vm,
                    ApiError::new("TypeError", format!("invalid {key}")),
                ))
            }
        }
    }
    result.validate().map_err(|e| error(vm, e))?;
    Ok(result)
}
impl WebHost {
    pub(super) fn new_promise(&self, vm: &mut Vm) -> std::result::Result<u64, JsError> {
        let native = vm.promise()?;
        let id = self
            .state
            .borrow_mut()
            .promise()
            .map_err(|e| error(vm, e))?;
        if let Some(Object {
            kind: Kind::Promise(p),
            ..
        }) = self.state.borrow_mut().objects.get_mut(&id)
        {
            p.native = native;
        }
        Ok(id)
    }
    pub(super) fn promise_value(&self, id: u64) -> JsValue {
        self.state
            .borrow()
            .objects
            .get(&id)
            .and_then(|o| match &o.kind {
                Kind::Promise(p) => Some(p.native.clone()),
                _ => None,
            })
            .unwrap_or(JsValue::Undefined)
    }
    pub(super) fn promise_token(&self, value: &JsValue) -> Option<u64> {
        self.state.borrow().objects.iter().find_map(|(id, o)| {
            matches!(&o.kind,Kind::Promise(p)if p.native==*value).then_some(*id)
        })
    }
    fn settle_now(
        &self,
        vm: &mut Vm,
        id: u64,
        ok: bool,
        value: JsValue,
    ) -> std::result::Result<(), JsError> {
        let native = self.promise_value(id);
        if ok {
            vm.resolve_promise(native, value)
        } else {
            vm.reject_promise(native, value)
        }
    }
    pub(super) fn allocate(
        &self,
        vm: &mut Vm,
        kind: Kind,
        class: &str,
    ) -> std::result::Result<JsValue, JsError> {
        let id = self
            .state
            .borrow_mut()
            .allocate(kind)
            .map_err(|e| error(vm, e))?;
        let value = wrap(vm, id, class)?;
        let constructor =
            self.state
                .borrow()
                .function(if class == "File" { "Blob" } else { class });
        let prototype = get(vm, constructor, "prototype")?;
        vm.set_host_prototype(&value, prototype)?;
        Ok(value)
    }
    pub(super) fn ready(
        &self,
        vm: &mut Vm,
        result: std::result::Result<JsValue, JsError>,
    ) -> std::result::Result<JsValue, JsError> {
        if let Ok(JsValue::String(text)) = &result {
            vm.check_string_bytes(text.len())?;
        }
        let native = vm.promise()?;
        let (ok, value) = match result {
            Ok(v) => (true, v),
            Err(e) => {
                if e.is_resource_limit() {
                    return Err(e);
                }
                (
                    false,
                    if let Some(v) = e.thrown {
                        v
                    } else {
                        vm.error_value(e)?
                    },
                )
            }
        };
        if ok {
            vm.resolve_promise(native.clone(), value)?;
        } else {
            vm.reject_promise(native.clone(), value)?;
        }
        Ok(native)
    }
    pub(super) fn command(
        &self,
        vm: &mut Vm,
        command: Command,
    ) -> std::result::Result<(), JsError> {
        self.state
            .borrow_mut()
            .enqueue(command)
            .map_err(|e| error(vm, e))
    }
    pub(super) fn permission(
        &self,
        vm: &mut Vm,
        name: &str,
    ) -> std::result::Result<JsValue, JsError> {
        let (url, profile, gesture) = {
            let s = self.state.borrow();
            (s.dom.url(), s.config.profile.clone(), s.gesture)
        };
        let id = self.new_promise(vm)?;
        let decision = if self.state.borrow().grants.contains(name) {
            Some("granted".into())
        } else {
            profile.permission(&url, name)
        };
        let decision = if !secure_context(&url) {
            Some("denied".into())
        } else {
            decision
        };
        if let Some(decision) = decision {
            self.state.borrow_mut().jobs.push_back(Job::Permission {
                id,
                name: name.into(),
                decision,
            });
        } else {
            self.state.borrow_mut().pending.insert(id);
            self.command(
                vm,
                Command::Permission {
                    id,
                    name: name.into(),
                    gesture,
                },
            )?;
        }
        Ok(self.promise_value(id))
    }
}
impl HostHooks for WebHost {
    fn activate_link(
        &mut self,
        vm: &mut Vm,
        href: &str,
        download: Option<&str>,
        target: &str,
    ) -> std::result::Result<bool, JsError> {
        let url =
            Url::parse(href).map_err(|e| error(vm, ApiError::new("TypeError", e.to_string())))?;
        let (document, gesture, blob) = {
            let state = self.state.borrow();
            (
                state.dom.url(),
                state.gesture,
                state.blob_urls.get(href).cloned(),
            )
        };
        if url.scheme() == "blob" && blob.is_none() {
            return Err(error(
                vm,
                ApiError::new(
                    "NotFoundError",
                    "Object URL was revoked or belongs to another realm",
                ),
            ));
        }
        if let Some(filename) = download.filter(|_| blob.is_some() || same_origin(&document, &url))
        {
            self.command(
                vm,
                Command::Download {
                    url,
                    filename: filename.chars().take(220).collect(),
                    bytes: blob,
                    gesture,
                },
            )?;
        } else if target.eq_ignore_ascii_case("_blank") {
            self.command(vm, Command::Popup { url, gesture })?;
        } else {
            self.navigate(vm, url, false)?;
        }
        Ok(true)
    }
    fn import_module(
        &mut self,
        vm: &mut Vm,
        url: &str,
        _referrer: &str,
    ) -> std::result::Result<JsValue, JsError> {
        let document = self.state.borrow().dom.url();
        let url = web_url(&document, url).map_err(|e| error(vm, e))?;
        let policy = Csp {
            inner: self.state.borrow().dom.policy(),
        };
        if !policy.allows("script-src", &document, &url, None) {
            let e = error(vm, ApiError::new("SecurityError", "module script policy"));
            return self.ready(vm, Err(e));
        }
        if self.state.borrow().pending.len() >= 128 {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "page pending service quota"),
            ));
        }
        let id = self.new_promise(vm)?;
        self.state.borrow_mut().pending.insert(id);
        self.command(
            vm,
            Command::ModuleLoad {
                known: vm.loaded_module_urls(),
                id,
                url,
                document,
                policy,
            },
        )?;
        Ok(self.promise_value(id))
    }
    fn after_operation(&mut self, vm: &mut Vm) -> std::result::Result<(), JsError> {
        self.collect_mutations(vm)
    }
    fn microtask(&mut self, vm: &mut Vm, token: u64) -> std::result::Result<HostResult, JsError> {
        self.observer_microtask(vm, token)
    }
    fn fork(&self) -> Option<Box<dyn HostHooks>> {
        Some(Box::new(self.clone()))
    }
    fn retained_bytes(&self) -> usize {
        let s = self.state.borrow();
        let mut bytes = s.objects.len() * 256;
        bytes = bytes.saturating_add(
            s.blob_urls
                .iter()
                .map(|(key, body)| key.len() + body.len() + 64)
                .sum::<usize>(),
        );
        bytes = bytes.saturating_add(s.jobs.len() * 256);
        for job in &s.jobs {
            let values = match job {
                Job::Callback {
                    function,
                    this,
                    args,
                } => vec![function.clone(), this.clone()]
                    .into_iter()
                    .chain(args.iter().cloned())
                    .collect::<Vec<_>>(),
                Job::Settle { value, .. } | Job::ReaderComplete { value, .. } => {
                    vec![value.clone()]
                }
                Job::Event { data, .. }
                | Job::DomEvent { data, .. }
                | Job::XhrEvent { data, .. } => vec![data.clone()],
                _ => Vec::new(),
            };
            bytes = bytes.saturating_add(
                values
                    .iter()
                    .map(|v| match v {
                        JsValue::String(s) => s.len(),
                        _ => 0,
                    })
                    .sum::<usize>(),
            );
        }
        bytes = bytes.saturating_add(s.geometry.len() * 128).saturating_add(
            s.commands
                .iter()
                .map(Command::retained_bytes)
                .sum::<usize>(),
        );
        bytes = bytes
            .saturating_add(
                s.canvases
                    .values()
                    .map(|c| {
                        c.pixels.len()
                            + c.path.iter().map(|p| p.len() * 16).sum::<usize>()
                            + c.state.len() * 512
                    })
                    .sum::<usize>(),
            )
            .saturating_add(s.images.values().map(|i| i.pixels.len()).sum::<usize>());
        for o in s.objects.values() {
            bytes = bytes.saturating_add(
                o.properties
                    .iter()
                    .map(|(key, v)| {
                        key.len()
                            + 64
                            + match v {
                                JsValue::String(s) => s.len(),
                                _ => 16,
                            }
                    })
                    .sum::<usize>(),
            );
            bytes = bytes.saturating_add(
                o.listeners
                    .iter()
                    .map(|(name, _, _)| name.len() + 128)
                    .sum::<usize>(),
            );
            bytes = bytes.saturating_add(match &o.kind {
                Kind::Request(r) => r.body.len(),
                Kind::Response { response, .. } => response.body.len(),
                Kind::Blob { bytes, .. } => bytes.len(),
                Kind::Idb(value) => value.retained_bytes(),
                Kind::Observer(observer) => observer.retained_bytes(),
                Kind::Headers(h) => h.0.iter().map(|(k, v)| k.len() + v.len() + 64).sum(),
                Kind::Params { values, .. } => {
                    values.iter().map(|(k, v)| k.len() + v.len() + 64).sum()
                }
                Kind::FormData(values) => values
                    .iter()
                    .map(|(key, value)| {
                        key.len()
                            + 64
                            + match value {
                                FormValue::Text(t) => t.len(),
                                FormValue::Blob { bytes, mime, name } => {
                                    bytes.len() + mime.len() + name.len()
                                }
                            }
                    })
                    .sum(),
                Kind::Xhr(x) => {
                    x.request.as_ref().map_or(0, |r| r.body.len())
                        + x.response.as_ref().map_or(0, |r| r.body.len())
                }
                Kind::Promise(p) => p.result.as_ref().map_or(0, |(_, v)| match v {
                    JsValue::String(s) => s.len(),
                    _ => 0,
                }),
                _ => 0,
            });
        }
        bytes
    }
    fn get(
        &mut self,
        vm: &mut Vm,
        id: u64,
        key: &str,
    ) -> std::result::Result<Option<JsValue>, JsError> {
        if let Some(value) = self.canvas_get(vm, id, key)? {
            return Ok(Some(value));
        }
        if id == WINDOW {
            let object = match key {
                "location" => Some((LOCATION, "Location")),
                "history" => Some((HISTORY, "History")),
                "navigator" => Some((NAVIGATOR, "Navigator")),
                "localStorage" => Some((LOCAL, "Storage")),
                "sessionStorage" => Some((SESSION, "Storage")),
                "indexedDB" => Some((INDEXED_DB, "IDBFactory")),
                _ => None,
            };
            if let Some((id, class)) = object {
                return Ok(Some(wrap(vm, id, class)?));
            }
        }
        if id == 0 {
            match key {
                "location" => return wrap(vm, LOCATION, "Location").map(Some),
                "fullscreenElement" => {
                    let s = self.state.borrow();
                    return Ok(Some(
                        s.fullscreen
                            .map(|n| wrap(vm, s.dom.node_host(n), "Element"))
                            .transpose()?
                            .unwrap_or(JsValue::Null),
                    ));
                }
                "fullscreenEnabled" => {
                    return Ok(Some(JsValue::Bool(
                        self.state.borrow().config.platform.is_some(),
                    )))
                }
                "exitFullscreen" => {
                    return Ok(Some(self.state.borrow().function("fullscreen:exit")))
                }
                _ => {}
            }
        }
        if key == "requestFullscreen"
            && self.state.borrow().dom.node_id(id).is_some_and(|node| {
                self.state.borrow().dom.with_document(|d| {
                    matches!(d.get(node).data, dom_bindings::html::NodeData::Element(_))
                })
            })
        {
            return Ok(Some(self.state.borrow().function("fullscreen:request")));
        }
        if id == 0 && key == "cookie" {
            let s = self.state.borrow();
            let url = s.dom.url();
            return Ok(Some(JsValue::string(
                s.config.cookies.header(&url, &url, true),
            )));
        }
        if id < WEB_BASE || id == WINDOW {
            return Ok(None);
        }
        if let Some(value) = self.database_get(vm, id, key)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.observer_get(vm, id, key)? {
            return Ok(Some(value));
        }
        let s = self.state.borrow();
        let Some(o) = s.objects.get(&id) else {
            return Err(error(
                vm,
                ApiError::new("InvalidStateError", "expired web object"),
            ));
        };
        let function = |name: &str| Some(s.function(name));
        let v = match &o.kind {
            Kind::Location => {
                let url = s.dom.url();
                match key {
                    "href" => Some(JsValue::string(url.as_str())),
                    "origin" => Some(JsValue::string(url.origin().ascii_serialization())),
                    "protocol" => Some(JsValue::string(format!("{}:", url.scheme()))),
                    "host" => Some(JsValue::string(
                        &url[url::Position::BeforeHost..url::Position::AfterPort],
                    )),
                    "hostname" => Some(JsValue::string(url.host_str().unwrap_or(""))),
                    "port" => Some(JsValue::string(
                        url.port().map_or_else(String::new, |n| n.to_string()),
                    )),
                    "pathname" => Some(JsValue::string(url.path())),
                    "search" => Some(JsValue::string(
                        url.query().map_or_else(String::new, |q| format!("?{q}")),
                    )),
                    "hash" => Some(JsValue::string(
                        url.fragment().map_or_else(String::new, |q| format!("#{q}")),
                    )),
                    "assign" => function("location:assign"),
                    "replace" => function("location:replace"),
                    "reload" => function("location:reload"),
                    "toString" => function("location:toString"),
                    _ => None,
                }
            }
            Kind::History => match key {
                "length" => Some(JsValue::Number(s.history.entries.len() as f64)),
                "state" => {
                    let data = s.history.current().state.clone();
                    drop(s);
                    return Ok(Some(if data.is_null() {
                        JsValue::Null
                    } else {
                        let clone: CloneData = serde_json::from_value(data).map_err(|e| {
                            error(vm, ApiError::new("DataCloneError", e.to_string()))
                        })?;
                        vm.deserialize_clone(&clone)?
                    }));
                }
                "pushState" => function("history:pushState"),
                "replaceState" => function("history:replaceState"),
                "back" => function("history:back"),
                "forward" => function("history:forward"),
                "go" => function("history:go"),
                _ => None,
            },
            Kind::Navigator => match key {
                "userAgent" => Some(JsValue::string(net::client::USER_AGENT_VALUE)),
                "language" => Some(JsValue::string(&s.config.language)),
                "languages" => Some(vm.array(vec![JsValue::string(&s.config.language)])?),
                "onLine" | "cookieEnabled" => Some(JsValue::Bool(true)),
                "clipboard" => Some(wrap(vm, CLIPBOARD, "Clipboard")?),
                "geolocation" => Some(wrap(vm, GEO, "Geolocation")?),
                "permissions" => Some(wrap(vm, PERMISSIONS, "Permissions")?),
                "storage" => Some(wrap(vm, STORAGE_MANAGER, "StorageManager")?),
                _ => None,
            },
            Kind::Storage(session) => {
                let values = s
                    .config
                    .profile
                    .storage(
                        &s.dom.url(),
                        if *session {
                            Some(&s.config.session)
                        } else {
                            None
                        },
                    )
                    .map_err(|e| error(vm, e))?;
                match key {
                    "length" => Some(JsValue::Number(values.len() as f64)),
                    "getItem" => function("storage:getItem"),
                    "setItem" => function("storage:setItem"),
                    "removeItem" => function("storage:removeItem"),
                    "clear" => function("storage:clear"),
                    "key" => function("storage:key"),
                    _ => values.get(key).map(JsValue::string),
                }
            }
            Kind::Headers(_) | Kind::HeaderView { .. } => match key {
                "append" => function("headers:append"),
                "set" => function("headers:set"),
                "get" => function("headers:get"),
                "has" => function("headers:has"),
                "delete" => function("headers:delete"),
                "entries" => function("headers:entries"),
                "keys" => function("headers:keys"),
                "values" => function("headers:values"),
                _ => None,
            },
            Kind::Promise(_) => None,
            Kind::Request(r) => match key {
                "bodyUsed" => Some(JsValue::Bool(s.used_bodies.contains(&id))),
                "url" => Some(JsValue::string(r.url.as_str())),
                "method" => Some(JsValue::string(&r.method)),
                "headers" => {
                    drop(s);
                    return self.header_view(vm, id, false).map(Some);
                }
                "credentials" => Some(JsValue::string(match r.credentials {
                    Credentials::Omit => "omit",
                    Credentials::SameOrigin => "same-origin",
                    Credentials::Include => "include",
                })),
                _ => body_property(key, &function),
            },
            Kind::Response { response: r, used } => match key {
                "url" => Some(JsValue::string(
                    if matches!(r.kind.as_str(), "opaque" | "opaqueredirect") {
                        ""
                    } else {
                        r.url.as_str()
                    },
                )),
                "status" => Some(JsValue::Number(r.status as f64)),
                "statusText" => Some(JsValue::string(&r.status_text)),
                "ok" => Some(JsValue::Bool((200..300).contains(&r.status))),
                "type" => Some(JsValue::string(&r.kind)),
                "redirected" => Some(JsValue::Bool(r.redirected)),
                "bodyUsed" => Some(JsValue::Bool(*used)),
                "headers" => {
                    let immutable = r.kind != "default";
                    drop(s);
                    return self.header_view(vm, id, immutable).map(Some);
                }
                _ => body_property(key, &function),
            },
            Kind::Url(url) => match key {
                "href" => Some(JsValue::string(url.as_str())),
                "origin" => Some(JsValue::string(url.origin().ascii_serialization())),
                "protocol" => Some(JsValue::string(format!("{}:", url.scheme()))),
                "hostname" => Some(JsValue::string(url.host_str().unwrap_or(""))),
                "pathname" => Some(JsValue::string(url.path())),
                "hash" => Some(JsValue::string(
                    url.fragment().map_or_else(String::new, |q| format!("#{q}")),
                )),
                "search" => Some(JsValue::string(
                    url.query().map_or_else(String::new, |q| format!("?{q}")),
                )),
                "toString" | "toJSON" => function("url:toString"),
                "searchParams" => {
                    let values = url
                        .query_pairs()
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect();
                    drop(s);
                    return Ok(Some(self.allocate(
                        vm,
                        Kind::Params {
                            values,
                            owner: Some(id),
                        },
                        "URLSearchParams",
                    )?));
                }
                _ => None,
            },
            Kind::Params { values, .. } => match key {
                "size" => Some(JsValue::Number(
                    s.params(id).map_or(values.len(), |(v, _)| v.len()) as f64,
                )),
                "append" => function("params:append"),
                "set" => function("params:set"),
                "get" => function("params:get"),
                "getAll" => function("params:getAll"),
                "has" => function("params:has"),
                "delete" => function("params:delete"),
                "sort" => function("params:sort"),
                "toString" => function("params:toString"),
                "entries" => function("params:entries"),
                _ => None,
            },
            Kind::FormData(_) => match key {
                "append" => function("form:append"),
                "set" => function("form:set"),
                "get" => function("form:get"),
                "getAll" => function("form:getAll"),
                "has" => function("form:has"),
                "delete" => function("form:delete"),
                _ => None,
            },
            Kind::Blob { bytes, mime } => match key {
                "size" => Some(JsValue::Number(bytes.len() as f64)),
                "type" => Some(JsValue::string(mime)),
                "text" => function("blob:text"),
                "slice" => function("blob:slice"),
                _ => None,
            },
            Kind::FileReader {
                ready,
                result,
                error,
            } => match key {
                "readyState" => Some(JsValue::Number(*ready as f64)),
                "result" => Some(result.clone()),
                "error" => Some(error.clone()),
                "readAsText" => function("reader:readAsText"),
                "readAsDataURL" => function("reader:readAsDataURL"),
                "abort" => function("reader:abort"),
                _ => None,
            },
            Kind::Xhr(x) => match key {
                "readyState" => Some(JsValue::Number(x.ready as f64)),
                "status" => Some(JsValue::Number(if x.ready >= 2 {
                    x.response.as_ref().map_or(0, |r| r.status)
                } else {
                    0
                } as f64)),
                "statusText" => Some(JsValue::string(if x.ready >= 2 {
                    x.response.as_ref().map_or("", |r| r.status_text.as_str())
                } else {
                    ""
                })),
                "responseURL" => Some(JsValue::string(
                    x.response.as_ref().map_or("", |r| r.url.as_str()),
                )),
                "responseText" => {
                    if !matches!(x.response_type.as_str(), "" | "text") {
                        return Err(error(
                            vm,
                            ApiError::new("InvalidStateError", "responseText type"),
                        ));
                    }
                    let response = x.response.clone();
                    let ready = x.ready;
                    let mime = x.mime_override.clone().or_else(|| {
                        response
                            .as_ref()
                            .and_then(|r| r.headers.get("content-type"))
                    });
                    drop(s);
                    return Ok(Some(JsValue::string(if ready >= 3 {
                        response.map_or_else(
                            || Ok(String::new()),
                            |r| decode_text(vm, &r.body, mime.as_deref()),
                        )?
                    } else {
                        String::new()
                    })));
                }
                "response" => {
                    let body = x.response.as_ref().map(|r| r.body.clone());
                    let kind = x.response_type.clone();
                    let ready = x.ready;
                    drop(s);
                    return Ok(Some(if ready < 4 {
                        JsValue::Null
                    } else if let Some(body) = body {
                        if kind == "json" {
                            match vm.parse_json(&String::from_utf8_lossy(&body)) {
                                Ok(value) => value,
                                Err(e) if e.is_resource_limit() => return Err(e),
                                Err(_) => JsValue::Null,
                            }
                        } else if kind == "blob" {
                            self.allocate(
                                vm,
                                Kind::Blob {
                                    bytes: body,
                                    mime: String::new(),
                                },
                                "Blob",
                            )?
                        } else {
                            let text = String::from_utf8_lossy(&body);
                            vm.check_string_bytes(text.len())?;
                            JsValue::string(text)
                        }
                    } else {
                        JsValue::Null
                    }));
                }
                "responseType" => Some(JsValue::string(&x.response_type)),
                "timeout" => Some(JsValue::Number(x.timeout as f64)),
                "withCredentials" => Some(JsValue::Bool(x.with_credentials)),
                "open" => function("xhr:open"),
                "send" => function("xhr:send"),
                "abort" => function("xhr:abort"),
                "setRequestHeader" => function("xhr:setRequestHeader"),
                "getResponseHeader" => function("xhr:getResponseHeader"),
                "getAllResponseHeaders" => function("xhr:getAllResponseHeaders"),
                "overrideMimeType" => function("xhr:overrideMimeType"),
                _ => None,
            },
            Kind::Worker { .. } => match key {
                "postMessage" => function("worker:postMessage"),
                "terminate" => function("worker:terminate"),
                _ => None,
            },
            Kind::Socket {
                ready,
                url,
                protocol,
                buffered,
            } => match key {
                "readyState" => Some(JsValue::Number(*ready as f64)),
                "url" => Some(JsValue::string(url.as_str())),
                "protocol" => Some(JsValue::string(protocol)),
                "bufferedAmount" => Some(JsValue::Number(*buffered as f64)),
                "send" => function("socket:send"),
                "close" => function("socket:close"),
                _ => None,
            },
            Kind::EventSource {
                ready,
                url,
                credentials,
            } => match key {
                "readyState" => Some(JsValue::Number(*ready as f64)),
                "url" => Some(JsValue::string(url.as_str())),
                "withCredentials" => Some(JsValue::Bool(*credentials)),
                "close" => function("sse:close"),
                _ => None,
            },
            Kind::Clipboard => match key {
                "readText" => function("clipboard:readText"),
                "writeText" => function("clipboard:writeText"),
                _ => None,
            },
            Kind::Geo => match key {
                "getCurrentPosition" => function("geo:getCurrentPosition"),
                _ => None,
            },
            Kind::Permissions => match key {
                "query" => function("permissions:query"),
                _ => None,
            },
            Kind::StorageManager => match key {
                "persist" => function("storageManager:persist"),
                "estimate" => function("storageManager:estimate"),
                _ => None,
            },
            Kind::IndexedDb
            | Kind::GeoRequest { .. }
            | Kind::Idb(_)
            | Kind::Observer(_)
            | Kind::CanvasContext { .. } => None,
            Kind::XhrOperation { .. } => None,
        };
        Ok(v.or_else(|| match key {
            "addEventListener" => function("events:add"),
            "removeEventListener" => function("events:remove"),
            _ => o.properties.get(key).cloned(),
        }))
    }
    fn set(
        &mut self,
        vm: &mut Vm,
        id: u64,
        key: &str,
        value: JsValue,
    ) -> std::result::Result<bool, JsError> {
        if self.canvas_set(vm, id, key, value.clone())? {
            return Ok(true);
        }
        if id == 0 && key == "cookie" {
            let value = vm.to_string(value)?;
            let s = self.state.borrow();
            let url = s.dom.url();
            s.config.cookies.set(&url, &url, &value, true);
            return Ok(true);
        }
        if matches!(id, WINDOW | 0) && key == "location"
            || id == LOCATION && matches!(key, "href" | "hash" | "search" | "pathname")
        {
            let value = vm.to_string(value)?;
            let old = self.state.borrow().dom.url();
            let url = if key == "hash" {
                let mut url = old.clone();
                url.set_fragment(Some(value.trim_start_matches('#')));
                url
            } else if key == "search" {
                let mut url = old.clone();
                url.set_query(Some(value.trim_start_matches('?')));
                url
            } else if key == "pathname" {
                let mut url = old.clone();
                url.set_path(&value);
                url
            } else {
                web_url(&old, &value).map_err(|e| error(vm, e))?
            };
            self.navigate(vm, url, false)?;
            return Ok(true);
        }
        if id < WEB_BASE || id == WINDOW {
            return Ok(false);
        }
        let storage = match self.state.borrow().objects.get(&id).map(|o| &o.kind) {
            Some(Kind::Storage(session)) => Some(*session),
            _ => None,
        };
        if let Some(session) = storage {
            let value = vm.to_string(value)?;
            let s = self.state.borrow();
            s.config
                .profile
                .storage_set(
                    &s.dom.url(),
                    if session {
                        Some(&s.config.session)
                    } else {
                        None
                    },
                    Some(key),
                    Some(&value),
                )
                .map_err(|e| error(vm, e))?;
            return Ok(true);
        }
        let string_value = if matches!(
            key,
            "responseType" | "href" | "hash" | "search" | "pathname"
        ) {
            Some(vm.to_string(value.clone())?)
        } else {
            None
        };
        let number = if key == "timeout" {
            Some(vm.to_number(value.clone())?)
        } else {
            None
        };
        let mut s = self.state.borrow_mut();
        let Some(o) = s.objects.get_mut(&id) else {
            return Ok(false);
        };
        if key.starts_with("on") {
            if o.properties.len() >= 1024 && !o.properties.contains_key(key) || key.len() > 1024 {
                return Err(error(
                    vm,
                    ApiError::new("QuotaExceededError", "event handler property quota"),
                ));
            }
            o.properties.insert(key.into(), value);
            return Ok(true);
        }
        match &mut o.kind {
            Kind::Xhr(x) => match key {
                "responseType" => {
                    let kind = string_value.unwrap_or_default();
                    if !matches!(kind.as_str(), "" | "text" | "json" | "blob") {
                        return Err(error(
                            vm,
                            ApiError::new("NotSupportedError", "response type"),
                        ));
                    }
                    if x.sent {
                        return Err(error(
                            vm,
                            ApiError::new("InvalidStateError", "request already sent"),
                        ));
                    }
                    x.response_type = kind;
                    return Ok(true);
                }
                "withCredentials" => {
                    if x.sent {
                        return Err(error(
                            vm,
                            ApiError::new("InvalidStateError", "request sent"),
                        ));
                    }
                    x.with_credentials = value.truthy();
                    return Ok(true);
                }
                "timeout" => {
                    x.timeout = number.unwrap_or(0.0).clamp(0.0, 60_000.0) as u64;
                    return Ok(true);
                }
                _ => {}
            },
            Kind::Url(url) => {
                match key {
                    "href" => {
                        *url = web_url(url, &string_value.unwrap_or_default())
                            .map_err(|e| error(vm, e))?
                    }
                    "hash" => url.set_fragment(Some(
                        string_value
                            .as_deref()
                            .unwrap_or("")
                            .trim_start_matches('#'),
                    )),
                    "search" => url.set_query(Some(
                        string_value
                            .as_deref()
                            .unwrap_or("")
                            .trim_start_matches('?'),
                    )),
                    "pathname" => url.set_path(string_value.as_deref().unwrap_or("")),
                    _ => return Ok(false),
                }
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
    fn call(
        &mut self,
        vm: &mut Vm,
        name: &str,
        this: JsValue,
        args: &[JsValue],
    ) -> std::result::Result<HostResult, JsError> {
        if name == "event:preventDefault" {
            vm.set(this, "defaultPrevented", JsValue::Bool(true))?;
            return Ok(HostResult::value(JsValue::Undefined));
        }
        if matches!(name, "db:listContains" | "db:listItem") {
            let value = if name == "db:listContains" {
                let key = text(vm, args, 0)?;
                JsValue::Bool(vm.array_values(this)?.contains(&JsValue::string(key)))
            } else {
                let index = vm.to_number(arg(args, 0))? as usize;
                vm.get(this, &index.to_string())?
            };
            return Ok(HostResult::value(value));
        }
        let id = vm.host_id(&this).unwrap_or(WINDOW);
        let value = self.invoke(vm, name, id, args)?;
        if matches!(name, "xhr:open" | "xhr:send") {
            return self.synchronous_event(
                vm,
                id,
                if name == "xhr:open" {
                    "readystatechange"
                } else {
                    "loadstart"
                },
                value,
            );
        }
        if name == "xhr:abort"
            && matches!(self.state.borrow().objects.get(&id).map(|o|&o.kind),Some(Kind::Xhr(x))if x.ready==4&&x.aborted)
        {
            let mut result = self.synchronous_event(vm, id, "readystatechange", value)?;
            for event in ["abort", "loadend"] {
                result.calls.extend(
                    self.synchronous_event(vm, id, event, JsValue::Undefined)?
                        .calls,
                );
            }
            result.finish = Some(id);
            return Ok(result);
        }
        Ok(HostResult::value(value))
    }
    fn finish(&mut self, _: &mut Vm, id: u64) -> std::result::Result<JsValue, JsError> {
        if let Some(Object {
            kind: Kind::Xhr(x), ..
        }) = self.state.borrow_mut().objects.get_mut(&id)
        {
            if x.aborted {
                x.ready = 0;
            }
        }
        Ok(JsValue::Undefined)
    }
    fn roots(&self, live: &HashSet<u64>) -> Vec<JsValue> {
        let s = self.state.borrow();
        let mut roots = s.functions.values().cloned().collect::<Vec<_>>();
        for (id, o) in &s.objects {
            if live.contains(id) || s.pending.contains(id) || *id < WEB_BASE + 100 {
                roots.extend(o.properties.values().cloned());
                roots.extend(o.listeners.iter().map(|(_, f, _)| f.clone()));
                match &o.kind {
                    Kind::Promise(p) => {
                        roots.push(p.native.clone());
                        if let Some((_, v)) = &p.result {
                            roots.push(v.clone());
                        }
                    }
                    Kind::FileReader { result, error, .. } => {
                        roots.extend([result.clone(), error.clone()])
                    }
                    Kind::GeoRequest { success, failure } => {
                        roots.extend([success.clone(), failure.clone()])
                    }
                    Kind::Idb(value) => roots.extend(value.roots()),
                    Kind::Observer(observer) => roots.push(observer.callback.clone()),
                    _ => {}
                }
            }
        }
        for job in &s.jobs {
            match job {
                Job::Callback {
                    function,
                    this,
                    args,
                } => {
                    roots.extend([function.clone(), this.clone()]);
                    roots.extend(args.clone());
                }
                Job::Event { data, .. } | Job::DomEvent { data, .. } => roots.push(data.clone()),
                Job::ReaderComplete { value, .. } => roots.push(value.clone()),
                Job::Settle { value, .. } => roots.push(value.clone()),
                Job::XhrEvent { data, .. } => roots.push(data.clone()),
                _ => {}
            }
        }
        roots
    }
    fn host_references(&self, live: &HashSet<u64>) -> Vec<u64> {
        let s = self.state.borrow();
        let mut refs = s
            .pending
            .iter()
            .copied()
            .chain(LOCATION..=INDEXED_DB)
            .collect::<Vec<_>>();
        refs.extend(
            s.fullscreen
                .into_iter()
                .chain(s.fullscreen_requests.values().copied())
                .map(|n| s.dom.node_host(n)),
        );
        for (id, o) in &s.objects {
            if let Kind::CanvasContext { node } = o.kind {
                if s.dom.with_document(|d| d.contains(0, node))
                    || live.contains(&s.dom.node_host(node))
                {
                    refs.push(*id);
                }
            }
            if let Kind::Observer(observer) = &o.kind {
                if observer.targets.keys().any(|&node| {
                    s.dom.with_document(|d| d.contains(0, node))
                        || live.contains(&s.dom.node_host(node))
                }) {
                    refs.push(*id);
                }
            }
            if live.contains(id) {
                if let Kind::Params {
                    owner: Some(owner), ..
                } = &o.kind
                {
                    refs.push(*owner);
                }
                if let Kind::Idb(value) = &o.kind {
                    refs.extend(value.references());
                }
                if let Kind::HeaderView { owner, .. } = o.kind {
                    refs.push(owner);
                }
                if let Kind::Observer(observer) = &o.kind {
                    refs.extend(observer.nodes().map(|node| s.dom.node_host(node)));
                }
                if let Kind::CanvasContext { node } = o.kind {
                    refs.push(s.dom.node_host(node));
                }
                if let Kind::XhrOperation { target } = o.kind {
                    refs.push(target);
                }
            }
        }
        for job in &s.jobs {
            match job {
                Job::Event { target, .. } => refs.push(*target),
                Job::XhrState { id, operation, .. } | Job::XhrEvent { id, operation, .. } => {
                    refs.extend([*id, *operation])
                }
                Job::ReaderComplete { id, .. } => refs.push(*id),
                Job::Settle { id, .. } => refs.push(*id),
                Job::Permission { id, .. } => refs.push(*id),
                Job::Database(task) => refs.extend(task.references()),
                Job::Observer { id } => refs.push(*id),
                _ => {}
            }
        }
        for (id, o) in &s.objects {
            if let Kind::HeaderView { owner, .. } = o.kind {
                if live.contains(&owner) {
                    refs.push(*id);
                }
            }
        }
        refs
    }
    fn sweep(&mut self, live: &HashSet<u64>) {
        let mut s = self.state.borrow_mut();
        let pending = s.pending.clone();
        s.objects
            .retain(|id, _| *id < WEB_BASE + 100 || live.contains(id) || pending.contains(id));
        s.used_bodies.retain(|id| live.contains(id));
        let handle = s.dom.clone();
        s.canvases.retain(|node, _| {
            handle.with_document(|d| d.contains(0, *node))
                || live.contains(&handle.node_host(*node))
        });
        let contexts = s.objects.keys().copied().collect::<HashSet<_>>();
        s.canvas_contexts.retain(|_, id| contexts.contains(id));
        s.images.retain(|node, _| {
            handle.with_document(|d| d.contains(0, *node))
                || live.contains(&handle.node_host(*node))
        });
    }
}
fn body_property(key: &str, f: &impl Fn(&str) -> Option<JsValue>) -> Option<JsValue> {
    match key {
        "text" => f("body:text"),
        "json" => f("body:json"),
        "blob" => f("body:blob"),
        "clone" => f("body:clone"),
        _ => None,
    }
}
impl WebHost {
    pub(super) fn navigate(
        &self,
        vm: &mut Vm,
        url: Url,
        replace: bool,
    ) -> std::result::Result<(), JsError> {
        let old = self.state.borrow().dom.url();
        let mut a = old.clone();
        a.set_fragment(None);
        let mut b = url.clone();
        b.set_fragment(None);
        if old == url && old.fragment().is_some() {
            return Ok(());
        }
        if a == b && old != url {
            let mut s = self.state.borrow_mut();
            let saved = s.history.current().state.clone();
            s.history
                .push(url.clone(), saved, replace)
                .map_err(|e| error(vm, e))?;
            s.dom.set_url(url.clone());
            let data = vm.object()?;
            vm.set(data.clone(), "oldURL", JsValue::string(old.as_str()))?;
            vm.set(data.clone(), "newURL", JsValue::string(url.as_str()))?;
            s.jobs.push_back(Job::DomEvent {
                name: "hashchange".into(),
                data,
            });
            s.enqueue(Command::SameDocument {
                url,
                replace,
                traversal: None,
            })
            .map_err(|e| error(vm, e))?;
        } else {
            let gesture = self.state.borrow().gesture;
            if gesture {
                self.command(vm, Command::Navigate { url, replace })?;
            } else {
                self.command(vm, Command::Notice("Blocked gestureless navigation".into()))?;
            }
        }
        Ok(())
    }
    pub(super) fn invoke(
        &self,
        vm: &mut Vm,
        name: &str,
        id: u64,
        args: &[JsValue],
    ) -> std::result::Result<JsValue, JsError> {
        if let Some(value) = self.utility(vm, name, id, args)? {
            return Ok(value);
        }
        if let Some(value) = self.database_call(vm, name, id, args)? {
            return Ok(value);
        }
        if let Some(value) = self.observer_call(vm, name, id, args)? {
            return Ok(value);
        }
        if let Some(value) = self.canvas_call(vm, name, id, args)? {
            return Ok(value);
        }
        match name {
            "DOMException" => {
                let message = text(vm, args, 0)?;
                let name = if args.len() > 1 {
                    text(vm, args, 1)?
                } else {
                    "Error".into()
                };
                let object = vm.object()?;
                vm.set(object.clone(), "name", JsValue::string(name))?;
                vm.set(object.clone(), "message", JsValue::string(message))?;
                return Ok(object);
            }
            "Headers" => {
                let h = headers(vm, arg(args, 0), &self.state)?;
                return self.allocate(vm, Kind::Headers(h), "Headers");
            }
            "Request" => {
                let r = request(vm, arg(args, 0), arg(args, 1), &self.state)?;
                return self.allocate(vm, Kind::Request(r), "Request");
            }
            "Response" => {
                let (body, mime) = body(vm, arg(args, 0), &self.state)?;
                let options = arg(args, 1);
                let v = get(vm, options.clone(), "status")?;
                let status = if v == JsValue::Undefined {
                    200
                } else {
                    vm.to_number(v)? as u16
                };
                if !(200..=599).contains(&status) {
                    return Err(error(vm, ApiError::new("RangeError", "response status")));
                }
                let v = get(vm, options.clone(), "headers")?;
                let mut h = headers(vm, v, &self.state)?;
                if let Some(mime) = mime {
                    if h.get("content-type").is_none() {
                        h.set("content-type", &mime).map_err(|e| error(vm, e))?;
                    }
                }
                let status_text = get(vm, options, "statusText")?;
                let status_text = if status_text == JsValue::Undefined {
                    String::new()
                } else {
                    vm.to_string(status_text)?
                };
                let url = self.state.borrow().dom.url();
                return self.allocate(
                    vm,
                    Kind::Response {
                        response: Response {
                            url,
                            status,
                            status_text,
                            headers: h,
                            body,
                            kind: "default".into(),
                            redirected: false,
                        },
                        used: false,
                    },
                    "Response",
                );
            }
            "fetch" => {
                let promise = self.new_promise(vm)?;
                let request = request(vm, arg(args, 0), arg(args, 1), &self.state);
                match request {
                    Ok(request) => {
                        let (document, policy) = {
                            let s = self.state.borrow();
                            (
                                s.dom.url(),
                                Csp {
                                    inner: s.dom.policy(),
                                },
                            )
                        };
                        if self.state.borrow().pending.len() >= 64 {
                            let e =
                                error(vm, ApiError::new("QuotaExceededError", "pending requests"));
                            let value = e.thrown.unwrap_or(JsValue::Undefined);
                            self.settle_now(vm, promise, false, value)?;
                        } else {
                            self.state.borrow_mut().pending.insert(promise);
                            self.command(
                                vm,
                                Command::Fetch {
                                    id: promise,
                                    request,
                                    document,
                                    policy,
                                },
                            )?;
                        }
                    }
                    Err(e) => {
                        if e.is_resource_limit() {
                            return Err(e);
                        }
                        let value = e.thrown.clone().unwrap_or(vm.error_value(e)?);
                        self.settle_now(vm, promise, false, value)?;
                    }
                }
                return Ok(self.promise_value(promise));
            }
            "body:text" | "body:json" | "body:blob" | "body:clone" => {
                let used = matches!(
                    self.state.borrow().objects.get(&id).map(|o| &o.kind),
                    Some(Kind::Response { used: true, .. })
                );
                if used || self.state.borrow().used_bodies.contains(&id) {
                    let e = error(vm, ApiError::new("TypeError", "body already used"));
                    if name == "body:clone" {
                        return Err(e);
                    }
                    return self.ready(vm, Err(e));
                }
                if name == "body:clone" {
                    let request =
                        self.state
                            .borrow()
                            .objects
                            .get(&id)
                            .and_then(|o| match &o.kind {
                                Kind::Request(r) => Some(r.clone()),
                                _ => None,
                            });
                    if let Some(request) = request {
                        return self.allocate(vm, Kind::Request(request), "Request");
                    }
                } else {
                    self.state.borrow_mut().used_bodies.insert(id);
                }
                let response = {
                    let mut s = self.state.borrow_mut();
                    match s.objects.get_mut(&id).map(|o| &mut o.kind) {
                        Some(Kind::Response { response, used }) => {
                            if name != "body:clone" {
                                *used = true;
                            }
                            Some(response.clone())
                        }
                        Some(Kind::Request(r)) => Some(Response {
                            url: r.url.clone(),
                            status: 200,
                            status_text: String::new(),
                            headers: r.headers.clone(),
                            body: r.body.clone(),
                            kind: "default".into(),
                            redirected: false,
                        }),
                        _ => None,
                    }
                };
                let response = response
                    .ok_or_else(|| error(vm, ApiError::new("TypeError", "body receiver")))?;
                if name == "body:clone" {
                    return self.allocate(
                        vm,
                        Kind::Response {
                            response,
                            used: false,
                        },
                        "Response",
                    );
                }
                let result = if name == "body:text" {
                    Ok(JsValue::string(String::from_utf8_lossy(&response.body)))
                } else if name == "body:json" {
                    vm.parse_json(&String::from_utf8_lossy(&response.body))
                } else {
                    self.allocate(
                        vm,
                        Kind::Blob {
                            bytes: response.body,
                            mime: response.headers.get("content-type").unwrap_or_default(),
                        },
                        "Blob",
                    )
                };
                return self.ready(vm, result);
            }
            "storage:getItem" | "storage:setItem" | "storage:removeItem" | "storage:clear"
            | "storage:key" => {
                let index = if name == "storage:key" {
                    Some(vm.to_number(arg(args, 0))?)
                } else {
                    None
                };
                let key = if matches!(name, "storage:clear" | "storage:key") {
                    String::new()
                } else {
                    text(vm, args, 0)?
                };
                let value = if name == "storage:setItem" {
                    Some(text(vm, args, 1)?)
                } else {
                    None
                };
                let s = self.state.borrow();
                let session = matches!(
                    s.objects.get(&id).map(|o| &o.kind),
                    Some(Kind::Storage(true))
                );
                if name == "storage:getItem" || name == "storage:key" {
                    let values = s
                        .config
                        .profile
                        .storage(
                            &s.dom.url(),
                            if session {
                                Some(&s.config.session)
                            } else {
                                None
                            },
                        )
                        .map_err(|e| error(vm, e))?;
                    return Ok(if name == "storage:getItem" {
                        values
                            .get(&key)
                            .map(JsValue::string)
                            .unwrap_or(JsValue::Null)
                    } else {
                        let n = index.unwrap_or(0.0);
                        let n = if n.is_finite() {
                            n.trunc().rem_euclid(4_294_967_296.0) as usize
                        } else {
                            0
                        };
                        values
                            .keys()
                            .nth(n)
                            .map(JsValue::string)
                            .unwrap_or(JsValue::Null)
                    });
                }
                s.config
                    .profile
                    .storage_set(
                        &s.dom.url(),
                        if session {
                            Some(&s.config.session)
                        } else {
                            None
                        },
                        if name == "storage:clear" {
                            None
                        } else {
                            Some(&key)
                        },
                        value.as_deref(),
                    )
                    .map_err(|e| error(vm, e))?;
                return Ok(JsValue::Undefined);
            }
            "location:toString" => {
                return Ok(JsValue::string(self.state.borrow().dom.url().as_str()))
            }
            "location:assign" | "location:replace" | "location:reload" => {
                let base = self.state.borrow().dom.url();
                let url = if name == "location:reload" {
                    base
                } else {
                    web_url(&base, &text(vm, args, 0)?).map_err(|e| error(vm, e))?
                };
                if name == "location:reload" {
                    if self.state.borrow().gesture {
                        self.command(vm, Command::Navigate { url, replace: true })?;
                    } else {
                        self.command(vm, Command::Notice("Blocked gestureless reload".into()))?;
                    }
                } else {
                    self.navigate(vm, url, name == "location:replace")?;
                }
                return Ok(JsValue::Undefined);
            }
            "history:pushState" | "history:replaceState" => {
                let data = vm
                    .serialize_clone(arg(args, 0))
                    .map_err(|e| error(vm, ApiError::new("DataCloneError", e.to_string())))?;
                let state = serde_json::to_value(data)
                    .map_err(|e| error(vm, ApiError::new("DataCloneError", e.to_string())))?;
                let base = self.state.borrow().dom.url();
                let url = if args.len() > 2
                    && !matches!(arg(args, 2), JsValue::Null | JsValue::Undefined)
                {
                    web_url(&base, &text(vm, args, 2)?).map_err(|e| error(vm, e))?
                } else {
                    base
                };
                let replace = name == "history:replaceState";
                let mut s = self.state.borrow_mut();
                s.history
                    .push(url.clone(), state, replace)
                    .map_err(|e| error(vm, e))?;
                s.dom.set_url(url.clone());
                s.enqueue(Command::SameDocument {
                    url,
                    replace,
                    traversal: None,
                })
                .map_err(|e| error(vm, e))?;
                return Ok(JsValue::Undefined);
            }
            "history:back" | "history:forward" | "history:go" => {
                let delta = if name == "history:back" {
                    -1
                } else if name == "history:forward" {
                    1
                } else {
                    vm.to_number(arg(args, 0))? as i32
                };
                if delta == 0 {
                    if self.state.borrow().gesture {
                        let url = self.state.borrow().dom.url();
                        self.command(vm, Command::Navigate { url, replace: true })?;
                    } else {
                        self.command(vm, Command::Notice("Blocked gestureless reload".into()))?;
                    }
                    return Ok(JsValue::Undefined);
                }
                let entry = self.state.borrow_mut().history.go(delta);
                if let Some(entry) = entry {
                    let value = if entry.state.is_null() {
                        JsValue::Null
                    } else {
                        let data: CloneData = serde_json::from_value(entry.state).map_err(|e| {
                            error(vm, ApiError::new("DataCloneError", e.to_string()))
                        })?;
                        vm.deserialize_clone(&data)?
                    };
                    let mut s = self.state.borrow_mut();
                    s.dom.set_url(entry.url.clone());
                    s.jobs.push_back(Job::DomEvent {
                        name: "popstate".into(),
                        data: value,
                    });
                    s.enqueue(Command::SameDocument {
                        url: entry.url,
                        replace: true,
                        traversal: Some(delta),
                    })
                    .map_err(|e| error(vm, e))?;
                }
                return Ok(JsValue::Undefined);
            }
            "XMLHttpRequest" => {
                return self.allocate(
                    vm,
                    Kind::Xhr(Box::new(Xhr {
                        operation: None,
                        mime_override: None,
                        request: None,
                        response: None,
                        ready: 0,
                        sent: false,
                        response_type: String::new(),
                        with_credentials: false,
                        timeout: 0,
                        aborted: false,
                    })),
                    "XMLHttpRequest",
                )
            }
            "xhr:open" => {
                let method = text(vm, args, 0)?;
                let input = text(vm, args, 1)?;
                let url =
                    web_url(&self.state.borrow().dom.url(), &input).map_err(|e| error(vm, e))?;
                if args.len() > 2 && !arg(args, 2).truthy() {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidAccessError", "synchronous XHR is unsupported"),
                    ));
                }
                let mut request = Request::get(url);
                request.method = method;
                request.validate().map_err(|e| error(vm, e))?;
                let mut s = self.state.borrow_mut();
                let Some(Object {
                    kind: Kind::Xhr(x), ..
                }) = s.objects.get_mut(&id)
                else {
                    return Err(error(vm, ApiError::new("TypeError", "XHR receiver")));
                };
                x.request = Some(request);
                x.response = None;
                x.ready = 1;
                x.sent = false;
                x.aborted = false;
                x.mime_override = None;
                let previous = x.operation.take();
                if let Some(operation) = previous {
                    s.pending.remove(&operation);
                    s.enqueue(Command::Abort { id: operation })
                        .map_err(|e| error(vm, e))?;
                }
                return Ok(JsValue::Undefined);
            }
            "xhr:setRequestHeader" | "xhr:overrideMimeType" => {
                let key = text(vm, args, 0)?;
                let value = if name == "xhr:setRequestHeader" {
                    text(vm, args, 1)?
                } else {
                    String::new()
                };
                let mut s = self.state.borrow_mut();
                let Some(Object {
                    kind: Kind::Xhr(x), ..
                }) = s.objects.get_mut(&id)
                else {
                    return Err(error(vm, ApiError::new("TypeError", "XHR receiver")));
                };
                if name == "xhr:overrideMimeType" {
                    if x.ready >= 3 {
                        return Err(error(
                            vm,
                            ApiError::new("InvalidStateError", "XHR response loading"),
                        ));
                    }
                    x.mime_override = Some(key);
                    return Ok(JsValue::Undefined);
                }
                if x.ready != 1 || x.sent {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "XHR not open"),
                    ));
                }
                if name == "xhr:setRequestHeader" && !Headers::request_forbidden(&key) {
                    if let Some(r) = &mut x.request {
                        r.headers.append(&key, &value).map_err(|e| error(vm, e))?;
                    }
                }
                return Ok(JsValue::Undefined);
            }
            "xhr:send" => {
                let (bytes, mime) = body(vm, arg(args, 0), &self.state)?;
                let mut s = self.state.borrow_mut();
                let document = s.dom.url();
                let policy = Csp {
                    inner: s.dom.policy(),
                };
                let Some(Object {
                    kind: Kind::Xhr(x), ..
                }) = s.objects.get_mut(&id)
                else {
                    return Err(error(vm, ApiError::new("TypeError", "XHR receiver")));
                };
                if x.ready != 1 || x.sent {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "XHR send state"),
                    ));
                }
                let mut request = x
                    .request
                    .clone()
                    .ok_or_else(|| error(vm, ApiError::new("InvalidStateError", "XHR not open")))?;
                if !matches!(request.method.as_str(), "GET" | "HEAD") {
                    request.body = bytes;
                    if let Some(mime) = mime {
                        if request.headers.get("content-type").is_none() {
                            request
                                .headers
                                .set("content-type", &mime)
                                .map_err(|e| error(vm, e))?;
                        }
                    }
                }
                request.credentials = if x.with_credentials {
                    Credentials::Include
                } else {
                    Credentials::SameOrigin
                };
                if x.timeout > 0 {
                    request.timeout_ms = x.timeout;
                }
                x.sent = true;
                if s.pending.len() >= 64 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "pending requests"),
                    ));
                }
                let operation = s
                    .allocate(Kind::XhrOperation { target: id })
                    .map_err(|e| error(vm, e))?;
                if let Some(Object {
                    kind: Kind::Xhr(x), ..
                }) = s.objects.get_mut(&id)
                {
                    x.operation = Some(operation);
                }
                s.pending.insert(operation);
                s.enqueue(Command::Fetch {
                    id: operation,
                    request,
                    document,
                    policy,
                })
                .map_err(|e| error(vm, e))?;
                return Ok(JsValue::Undefined);
            }
            "xhr:abort" => {
                let mut s = self.state.borrow_mut();
                if let Some(Object {
                    kind: Kind::Xhr(x), ..
                }) = s.objects.get_mut(&id)
                {
                    let active = x.sent || matches!(x.ready, 2 | 3);
                    x.ready = if active { 4 } else { 0 };
                    x.aborted = true;
                    x.sent = false;
                    x.response = None;
                    let operation = x.operation.take();
                    if let Some(operation) = operation {
                        s.pending.remove(&operation);
                        s.enqueue(Command::Abort { id: operation })
                            .map_err(|e| error(vm, e))?;
                    }
                }
                return Ok(JsValue::Undefined);
            }
            "xhr:getResponseHeader" | "xhr:getAllResponseHeaders" => {
                let key = if name == "xhr:getResponseHeader" {
                    text(vm, args, 0)?
                } else {
                    String::new()
                };
                let s = self.state.borrow();
                let r = match s.objects.get(&id).map(|o| &o.kind) {
                    Some(Kind::Xhr(x)) if x.ready >= 2 => x.response.as_ref(),
                    _ => None,
                };
                return Ok(if name == "xhr:getResponseHeader" {
                    r.and_then(|r| r.headers.get(&key))
                        .map(JsValue::string)
                        .unwrap_or(JsValue::Null)
                } else {
                    JsValue::string(r.map_or_else(String::new, |r| {
                        r.headers
                            .entries()
                            .into_iter()
                            .map(|(k, v)| format!("{k}: {v}\r\n"))
                            .collect::<String>()
                    }))
                });
            }
            "events:add" | "events:remove" => {
                let event = text(vm, args, 0)?;
                let callback = arg(args, 1);
                let once = get(vm, arg(args, 2), "once")?.truthy();
                let mut s = self.state.borrow_mut();
                let Some(o) = s.objects.get_mut(&id) else {
                    return Err(error(vm, ApiError::new("TypeError", "event target")));
                };
                if event.is_empty() {
                    return Ok(JsValue::Undefined);
                }
                if event.len() > 1024 || o.listeners.len() >= 1024 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "event listener quota"),
                    ));
                }
                if name == "events:remove" {
                    o.listeners
                        .retain(|(n, f, _)| n != &event || f != &callback);
                } else if vm.callable(&callback)
                    && !o
                        .listeners
                        .iter()
                        .any(|(n, f, _)| n == &event && f == &callback)
                {
                    o.listeners.push((event, callback, once));
                }
                return Ok(JsValue::Undefined);
            }
            "clipboard:readText" | "clipboard:writeText" => {
                let text = if name == "clipboard:writeText" {
                    Some(text(vm, args, 0)?)
                } else {
                    None
                };
                let id = self.new_promise(vm)?;
                let gesture = self.state.borrow().gesture;
                if !gesture || !secure_context(&self.state.borrow().dom.url()) {
                    let e = error(
                        vm,
                        ApiError::new(
                            "NotAllowedError",
                            "clipboard requires a secure context and user gesture",
                        ),
                    );
                    let value = e.thrown.unwrap_or(JsValue::Undefined);
                    self.settle_now(vm, id, false, value)?;
                    return Ok(self.promise_value(id));
                }
                self.state.borrow_mut().pending.insert(id);
                let permission = {
                    let s = self.state.borrow();
                    if s.grants.contains("clipboard-read") {
                        Some("granted".into())
                    } else {
                        s.config.profile.permission(&s.dom.url(), "clipboard-read")
                    }
                };
                self.command(
                    vm,
                    Command::Clipboard {
                        id,
                        text,
                        gesture,
                        permission,
                    },
                )?;
                return Ok(self.promise_value(id));
            }
            "notification:requestPermission" => return self.permission(vm, "notifications"),
            "permissions:query" => {
                let value = get(vm, arg(args, 0), "name")?;
                let name = vm.to_string(value)?;
                let s = self.state.borrow();
                if !matches!(
                    name.as_str(),
                    "geolocation"
                        | "notifications"
                        | "clipboard-read"
                        | "fullscreen"
                        | "persistent-storage"
                        | "camera"
                        | "microphone"
                ) {
                    drop(s);
                    let e = error(vm, ApiError::new("TypeError", "unknown permission name"));
                    return self.ready(vm, Err(e));
                }
                let permission = if matches!(name.as_str(), "camera" | "microphone") {
                    s.dom.console(format!("{name} denied: no v1 backend"));
                    "denied".into()
                } else if !secure_context(&s.dom.url()) {
                    "denied".into()
                } else if s.grants.contains(&name) {
                    "granted".into()
                } else {
                    s.config
                        .profile
                        .permission(&s.dom.url(), &name)
                        .unwrap_or_else(|| "prompt".into())
                };
                drop(s);
                let value = vm.object()?;
                vm.set(value.clone(), "state", JsValue::string(permission))?;
                return self.ready(vm, Ok(value));
            }
            "storageManager:estimate" => {
                let (usage, quota) = {
                    let s = self.state.borrow();
                    s.config
                        .profile
                        .estimate(&s.dom.url())
                        .map_err(|e| error(vm, e))?
                };
                let value = vm.object()?;
                vm.set(value.clone(), "quota", JsValue::Number(quota as f64))?;
                vm.set(value.clone(), "usage", JsValue::Number(usage as f64))?;
                return self.ready(vm, Ok(value));
            }
            "storageManager:persist" => return self.permission(vm, "persistent-storage"),
            "alert" | "confirm" | "prompt" => {
                let message = text(vm, args, 0)?;
                let default = if args.len() > 1 {
                    text(vm, args, 1)?
                } else {
                    String::new()
                };
                let (platform, url) = {
                    let s = self.state.borrow();
                    (s.config.platform.clone(), s.dom.url())
                };
                let platform = platform.ok_or_else(|| {
                    error(
                        vm,
                        ApiError::new("NotSupportedError", "dialog backend unavailable"),
                    )
                })?;
                let waiting = std::time::Instant::now();
                let result = platform.dialog(Dialog {
                    origin: origin(&url).map_err(|e| error(vm, e))?,
                    kind: match name {
                        "confirm" => DialogKind::Confirm,
                        "prompt" => DialogKind::Prompt,
                        _ => DialogKind::Alert,
                    },
                    message,
                    default,
                });
                vm.exclude_host_wait(waiting.elapsed());
                let result = result.map_err(|e| error(vm, e))?;
                return Ok(match name {
                    "confirm" => JsValue::Bool(result.is_some()),
                    "prompt" => result.map(JsValue::string).unwrap_or(JsValue::Null),
                    _ => JsValue::Undefined,
                });
            }
            "open" => {
                let input = text(vm, args, 0)?;
                let url =
                    web_url(&self.state.borrow().dom.url(), &input).map_err(|e| error(vm, e))?;
                let gesture = self.state.borrow().gesture;
                self.command(vm, Command::Popup { url, gesture })?;
                return Ok(JsValue::Null);
            }
            _ => {}
        }
        if name.starts_with("headers:") {
            let key = if matches!(name, "headers:entries" | "headers:keys" | "headers:values") {
                String::new()
            } else {
                text(vm, args, 0)?
            };
            let value = if matches!(name, "headers:set" | "headers:append") {
                Some(text(vm, args, 1)?)
            } else {
                None
            };
            let (mut h, owner, immutable) = self
                .state
                .borrow()
                .headers(id)
                .ok_or_else(|| error(vm, ApiError::new("TypeError", "Headers receiver")))?;
            let mutation = matches!(name, "headers:set" | "headers:append" | "headers:delete");
            if !matches!(name, "headers:entries" | "headers:keys" | "headers:values") {
                Headers::validate_name(&key).map_err(|e| error(vm, e))?;
            }
            if mutation && immutable {
                return Err(error(
                    vm,
                    ApiError::new("TypeError", "immutable response headers"),
                ));
            }
            let request = owner.is_some_and(|owner| {
                matches!(
                    self.state.borrow().objects.get(&owner).map(|o| &o.kind),
                    Some(Kind::Request(_))
                )
            });
            if mutation && request && Headers::request_forbidden(&key) {
                return Ok(JsValue::Undefined);
            }
            let value = match name {
                "headers:get" => h.get(&key).map(JsValue::string).unwrap_or(JsValue::Null),
                "headers:has" => JsValue::Bool(h.get(&key).is_some()),
                "headers:delete" => {
                    h.delete(&key);
                    JsValue::Undefined
                }
                "headers:set" => {
                    h.set(&key, value.as_deref().unwrap_or(""))
                        .map_err(|e| error(vm, e))?;
                    JsValue::Undefined
                }
                "headers:append" => {
                    h.append(&key, value.as_deref().unwrap_or(""))
                        .map_err(|e| error(vm, e))?;
                    JsValue::Undefined
                }
                _ => {
                    let entries = h.entries();
                    let mut values = Vec::new();
                    for (k, v) in entries {
                        values.push(if name == "headers:keys" {
                            JsValue::string(k)
                        } else if name == "headers:values" {
                            JsValue::string(v)
                        } else {
                            vm.array(vec![JsValue::string(k), JsValue::string(v)])?
                        });
                    }
                    return vm.array(values);
                }
            };
            if mutation {
                let mut s = self.state.borrow_mut();
                match owner
                    .and_then(|owner| s.objects.get_mut(&owner))
                    .map(|o| &mut o.kind)
                {
                    Some(Kind::Request(r)) => r.headers = h,
                    Some(Kind::Response { response, .. }) => response.headers = h,
                    _ => {
                        if let Some(Object {
                            kind: Kind::Headers(headers),
                            ..
                        }) = s.objects.get_mut(&id)
                        {
                            *headers = h;
                        }
                    }
                }
            }
            return Ok(value);
        }
        Err(error(
            vm,
            ApiError::new(
                "NotSupportedError",
                format!("unsupported web method {name}"),
            ),
        ))
    }
}
impl WebHost {
    fn synchronous_event(
        &self,
        vm: &mut Vm,
        id: u64,
        name: &str,
        value: JsValue,
    ) -> std::result::Result<HostResult, JsError> {
        let callbacks = {
            let mut s = self.state.borrow_mut();
            let Some(object) = s.objects.get_mut(&id) else {
                return Ok(HostResult::value(value));
            };
            let mut callbacks = object
                .properties
                .get(&format!("on{name}"))
                .filter(|v| vm.callable(v))
                .cloned()
                .into_iter()
                .collect::<Vec<_>>();
            callbacks.extend(
                object
                    .listeners
                    .iter()
                    .filter(|(n, _, _)| n == name)
                    .map(|(_, f, _)| f.clone()),
            );
            object.listeners.retain(|(n, _, once)| n != name || !*once);
            callbacks
        };
        let target = wrap(vm, id, "XMLHttpRequest")?;
        let event = vm.object()?;
        vm.set(event.clone(), "target", target.clone())?;
        vm.set(event.clone(), "currentTarget", target.clone())?;
        vm.set(event.clone(), "type", JsValue::string(name))?;
        for (name, value) in [
            ("loaded", JsValue::Number(0.0)),
            ("total", JsValue::Number(0.0)),
            ("lengthComputable", JsValue::Bool(false)),
        ] {
            vm.set(event.clone(), name, value)?;
        }
        let mut result = HostResult::value(value);
        result.catch_callback_errors = true;
        result.calls = callbacks
            .into_iter()
            .map(|callback| (callback, target.clone(), vec![event.clone()]))
            .collect();
        Ok(result)
    }
    fn header_view(
        &self,
        vm: &mut Vm,
        owner: u64,
        immutable: bool,
    ) -> std::result::Result<JsValue, JsError> {
        let existing = self.state.borrow().objects.iter().find_map(|(id, o)| {
            matches!(o.kind,Kind::HeaderView{owner:o,..}if o==owner).then_some(*id)
        });
        if let Some(id) = existing {
            return wrap(vm, id, "Headers");
        }
        self.allocate(vm, Kind::HeaderView { owner, immutable }, "Headers")
    }
}
fn decode_text(
    vm: &mut Vm,
    bytes: &[u8],
    mime: Option<&str>,
) -> std::result::Result<String, JsError> {
    let encoding = mime
        .and_then(|mime| {
            mime.split(';').skip(1).find_map(|p| {
                p.trim()
                    .split_once('=')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("charset"))
                    .and_then(|(_, v)| {
                        encoding_rs::Encoding::for_label(v.trim_matches(['\'', '"']).as_bytes())
                    })
            })
        })
        .unwrap_or(encoding_rs::UTF_8);
    let text = encoding.decode(bytes).0;
    vm.check_string_bytes(text.len())?;
    Ok(text.into_owned())
}
