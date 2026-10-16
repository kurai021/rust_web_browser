use super::*;
use base64::Engine;
impl WebHost {
    pub(super) fn utility(
        &self,
        vm: &mut Vm,
        name: &str,
        id: u64,
        args: &[JsValue],
    ) -> std::result::Result<Option<JsValue>, JsError> {
        let snapshot = || self.state.borrow().objects.get(&id).map(|o| o.kind.clone());
        let value = match name {
            "fullscreen:request" => {
                let (node, gesture, secure, platform) = {
                    let s = self.state.borrow();
                    (
                        s.dom.node_id(id),
                        s.gesture,
                        secure_context(&s.dom.url()),
                        s.config.platform.is_some(),
                    )
                };
                if !gesture || !secure || !platform {
                    let e = error(
                        vm,
                        ApiError::new(
                            "NotAllowedError",
                            "fullscreen requires a secure context, native window and user gesture",
                        ),
                    );
                    return self.ready(vm, Err(e)).map(Some);
                }
                let node = node
                    .ok_or_else(|| error(vm, ApiError::new("TypeError", "fullscreen element")))?;
                let promise = self.permission(vm, "fullscreen")?;
                if let Some(id) = vm.host_id(&promise) {
                    self.state.borrow_mut().fullscreen_requests.insert(id, node);
                }
                promise
            }
            "fullscreen:exit" => {
                let active = self.state.borrow_mut().fullscreen.take().is_some();
                if active {
                    self.command(vm, Command::Fullscreen { enter: false })?;
                    self.state.borrow_mut().jobs.push_back(Job::DomEvent {
                        name: "fullscreenchange".into(),
                        data: JsValue::Undefined,
                    });
                }
                self.ready(vm, Ok(JsValue::Undefined))?
            }
            "URL" => {
                let input = text(vm, args, 0)?;
                let url = if args.len() > 1 {
                    let base = text(vm, args, 1)?;
                    Url::parse(&base).and_then(|b| b.join(&input))
                } else {
                    Url::parse(&input)
                }
                .map_err(|e| error(vm, ApiError::new("TypeError", e.to_string())))?;
                self.allocate(vm, Kind::Url(url), "URL")?
            }
            "url:toString" => match snapshot() {
                Some(Kind::Url(url)) => JsValue::string(url.as_str()),
                _ => return Err(error(vm, ApiError::new("TypeError", "URL receiver"))),
            },
            "URLSearchParams" => {
                let input = arg(args, 0);
                let old = vm
                    .host_id(&input)
                    .and_then(|id| self.state.borrow().params(id));
                let values = if let Some((values, _)) = old {
                    values
                } else if matches!(input, JsValue::Undefined | JsValue::Null) {
                    Vec::new()
                } else if matches!(input, JsValue::String(_)) {
                    let value = vm.to_string(input)?;
                    url::form_urlencoded::parse(value.trim_start_matches('?').as_bytes())
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect()
                } else if matches!(get(vm, input.clone(), "length")?, JsValue::Number(_)) {
                    let mut values = Vec::new();
                    for entry in vm.array_values(input)? {
                        let k = vm.get(entry.clone(), "0")?;
                        let v = vm.get(entry, "1")?;
                        values.push((vm.to_string(k)?, vm.to_string(v)?));
                    }
                    values
                } else {
                    let mut values = Vec::new();
                    for key in vm.keys(&input, true)? {
                        if let dom_bindings::js::PropertyKey::String(k) = key {
                            let v = vm.get(input.clone(), &k)?;
                            values.push((k, vm.to_string(v)?));
                        }
                    }
                    values
                };
                self.allocate(
                    vm,
                    Kind::Params {
                        values,
                        owner: None,
                    },
                    "URLSearchParams",
                )?
            }
            "Blob" => {
                let mut bytes = Vec::new();
                if arg(args, 0) != JsValue::Undefined {
                    for part in vm.array_values(arg(args, 0))? {
                        let data = vm.host_id(&part).and_then(|id| {
                            self.state.borrow().objects.get(&id).map(|o| o.kind.clone())
                        });
                        let data = if let Some(Kind::Blob { bytes, .. }) = data {
                            bytes
                        } else {
                            vm.to_string(part)?.into_bytes()
                        };
                        if bytes.len().saturating_add(data.len()) > MAX_BODY {
                            return Err(error(
                                vm,
                                ApiError::new("QuotaExceededError", "Blob quota"),
                            ));
                        }
                        bytes.extend_from_slice(&data);
                    }
                }
                let value = get(vm, arg(args, 1), "type")?;
                let mime = if value == JsValue::Undefined {
                    String::new()
                } else {
                    vm.to_string(value)?.to_ascii_lowercase()
                };
                let mime = if mime.bytes().all(|b| (32..127).contains(&b)) {
                    mime
                } else {
                    String::new()
                };
                self.allocate(vm, Kind::Blob { bytes, mime }, "Blob")?
            }
            "blob:text" => {
                let Some(Kind::Blob { bytes, .. }) = snapshot() else {
                    return Err(error(vm, ApiError::new("TypeError", "Blob receiver")));
                };
                self.ready(vm, Ok(JsValue::string(String::from_utf8_lossy(&bytes))))?
            }
            "blob:slice" => {
                let Some(Kind::Blob { bytes, .. }) = snapshot() else {
                    return Err(error(vm, ApiError::new("TypeError", "Blob receiver")));
                };
                let len = bytes.len();
                let n = vm.to_number(arg(args, 0))?;
                let start = if n < 0.0 {
                    (len as f64 + n).max(0.0) as usize
                } else {
                    (n as usize).min(len)
                };
                let n = if args.len() > 1 {
                    vm.to_number(arg(args, 1))?
                } else {
                    len as f64
                };
                let end = if n < 0.0 {
                    (len as f64 + n).max(0.0) as usize
                } else {
                    (n as usize).min(len)
                };
                let mime = if args.len() > 2 {
                    text(vm, args, 2)?.to_ascii_lowercase()
                } else {
                    String::new()
                };
                let mime = if mime.bytes().all(|b| (32..127).contains(&b)) {
                    mime
                } else {
                    String::new()
                };
                self.allocate(
                    vm,
                    Kind::Blob {
                        bytes: bytes[start..end.max(start)].to_vec(),
                        mime,
                    },
                    "Blob",
                )?
            }
            "FileReader" => self.allocate(
                vm,
                Kind::FileReader {
                    ready: 0,
                    result: JsValue::Null,
                    error: JsValue::Null,
                },
                "FileReader",
            )?,
            "reader:readAsText" | "reader:readAsDataURL" => {
                let input = arg(args, 0);
                let blob = vm
                    .host_id(&input)
                    .and_then(|i| self.state.borrow().objects.get(&i).map(|o| o.kind.clone()));
                let Some(Kind::Blob { bytes, mime }) = blob else {
                    return Err(error(vm, ApiError::new("TypeError", "expected Blob")));
                };
                let value = if name == "reader:readAsText" {
                    String::from_utf8_lossy(&bytes).into_owned()
                } else {
                    format!(
                        "data:{};base64,{}",
                        if mime.is_empty() {
                            "application/octet-stream"
                        } else {
                            &mime
                        },
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    )
                };
                vm.check_string_bytes(value.len())?;
                let mut s = self.state.borrow_mut();
                let Some(Object {
                    kind:
                        Kind::FileReader {
                            ready,
                            result,
                            error: e,
                        },
                    ..
                }) = s.objects.get_mut(&id)
                else {
                    return Err(error(vm, ApiError::new("TypeError", "FileReader receiver")));
                };
                if *ready == 1 {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "FileReader busy"),
                    ));
                }
                *ready = 1;
                *result = JsValue::Null;
                *e = JsValue::Null;
                s.jobs.push_back(Job::Event {
                    target: id,
                    name: "loadstart".into(),
                    data: JsValue::Undefined,
                });
                s.jobs.push_back(Job::ReaderComplete {
                    id,
                    value: JsValue::string(value),
                });
                JsValue::Undefined
            }
            "reader:abort" => {
                let mut s = self.state.borrow_mut();
                if let Some(Object {
                    kind: Kind::FileReader { ready, result, .. },
                    ..
                }) = s.objects.get_mut(&id)
                {
                    *ready = 2;
                    *result = JsValue::Null;
                }
                s.jobs
                    .retain(|job| !matches!(job,Job::ReaderComplete{id:i,..} if *i==id));
                s.jobs.push_back(Job::Event {
                    target: id,
                    name: "abort".into(),
                    data: JsValue::Undefined,
                });
                s.jobs.push_back(Job::Event {
                    target: id,
                    name: "loadend".into(),
                    data: JsValue::Undefined,
                });
                JsValue::Undefined
            }
            "FormData" => {
                let mut values = Vec::new();
                let input = arg(args, 0);
                if input != JsValue::Undefined {
                    let handle = self.state.borrow().dom.clone();
                    let node = vm
                        .host_id(&input)
                        .and_then(|id| handle.node_id(id))
                        .ok_or_else(|| error(vm, ApiError::new("TypeError", "expected form")))?;
                    let doc = handle.document();
                    if !doc.is_element_named(node, "form") {
                        return Err(error(vm, ApiError::new("TypeError", "expected form")));
                    }
                    for control in doc.elements_from(node) {
                        if doc.get_attribute(control, "disabled").is_some() {
                            continue;
                        }
                        let Some(name) =
                            doc.get_attribute(control, "name").filter(|s| !s.is_empty())
                        else {
                            continue;
                        };
                        if matches!(
                            doc.get_attribute(control, "type"),
                            Some("checkbox" | "radio")
                        ) && !doc.control_checked(control)
                        {
                            continue;
                        }
                        if ["input", "textarea", "select"]
                            .iter()
                            .any(|tag| doc.is_element_named(control, tag))
                        {
                            values.push((
                                name.into(),
                                FormValue::Text(doc.control_value(control).into()),
                            ));
                        }
                    }
                }
                self.allocate(vm, Kind::FormData(values), "FormData")?
            }
            "Worker" => {
                let input = text(vm, args, 0)?;
                let url =
                    web_url(&self.state.borrow().dom.url(), &input).map_err(|e| error(vm, e))?;
                let document = self.state.borrow().dom.url();
                let policy = Csp {
                    inner: self.state.borrow().dom.policy(),
                };
                if !same_origin(&document, &url)
                    || !policy.allows("worker-src", &document, &url, None)
                {
                    return Err(error(
                        vm,
                        ApiError::new("SecurityError", "worker origin/policy"),
                    ));
                }
                if self
                    .state
                    .borrow()
                    .objects
                    .values()
                    .filter(|o| matches!(o.kind, Kind::Worker { alive: true }))
                    .count()
                    >= 4
                {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "worker limit"),
                    ));
                }
                let id = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::Worker { alive: true })
                    .map_err(|e| error(vm, e))?;
                self.state.borrow_mut().pending.insert(id);
                self.command(vm, Command::WorkerStart { id, url, policy })?;
                wrap(vm, id, "Worker")?
            }
            "worker:postMessage" => {
                if !matches!(snapshot(), Some(Kind::Worker { alive: true })) {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "terminated worker"),
                    ));
                }
                let data = vm
                    .serialize_clone(arg(args, 0))
                    .map_err(|e| error(vm, ApiError::new("DataCloneError", e.to_string())))?;
                self.command(vm, Command::WorkerMessage { id, data })?;
                JsValue::Undefined
            }
            "worker:terminate" => {
                let mut s = self.state.borrow_mut();
                if let Some(Object {
                    kind: Kind::Worker { alive },
                    ..
                }) = s.objects.get_mut(&id)
                {
                    *alive = false;
                }
                s.pending.remove(&id);
                s.enqueue(Command::WorkerStop { id })
                    .map_err(|e| error(vm, e))?;
                JsValue::Undefined
            }
            "WebSocket" => {
                let value = text(vm, args, 0)?;
                let base = self.state.borrow().dom.url();
                let mut url = base
                    .join(&value)
                    .map_err(|e| error(vm, ApiError::new("SyntaxError", e.to_string())))?;
                if url.scheme() == "http" {
                    let _ = url.set_scheme("ws");
                } else if url.scheme() == "https" {
                    let _ = url.set_scheme("wss");
                }
                if !matches!(url.scheme(), "ws" | "wss")
                    || url.fragment().is_some()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(error(
                        vm,
                        ApiError::new("SyntaxError", "invalid WebSocket URL"),
                    ));
                }
                let protocols = if arg(args, 1) == JsValue::Undefined {
                    Vec::new()
                } else if matches!(arg(args, 1), JsValue::String(_)) {
                    vec![text(vm, args, 1)?]
                } else {
                    let mut out = Vec::new();
                    for v in vm.array_values(arg(args, 1))? {
                        out.push(vm.to_string(v)?);
                    }
                    out
                };
                let mut seen = HashSet::new();
                for p in &protocols {
                    Headers::validate_name(p).map_err(|e| error(vm, e))?;
                    if !seen.insert(p) {
                        return Err(error(
                            vm,
                            ApiError::new("SyntaxError", "duplicate protocol"),
                        ));
                    }
                }
                if protocols.len() > 16 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "protocol limit"),
                    ));
                }
                let policy = Csp {
                    inner: self.state.borrow().dom.policy(),
                };
                if !policy.allows("connect-src", &base, &url, None) {
                    return Err(error(vm, ApiError::new("SecurityError", "socket policy")));
                }
                let id = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::Socket {
                        ready: 0,
                        protocol: String::new(),
                        buffered: 0,
                        url: url.clone(),
                    })
                    .map_err(|e| error(vm, e))?;
                self.state.borrow_mut().pending.insert(id);
                self.command(
                    vm,
                    Command::SocketOpen {
                        id,
                        url,
                        protocols,
                        document: base,
                        policy,
                    },
                )?;
                wrap(vm, id, "WebSocket")?
            }
            "socket:send" => {
                let Some(Kind::Socket { ready, .. }) = snapshot() else {
                    return Err(error(vm, ApiError::new("TypeError", "WebSocket receiver")));
                };
                if ready == 0 {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "socket connecting"),
                    ));
                }
                if ready != 1 {
                    return Ok(Some(JsValue::Undefined));
                }
                let input = arg(args, 0);
                let blob = vm
                    .host_id(&input)
                    .and_then(|i| self.state.borrow().objects.get(&i).map(|o| o.kind.clone()));
                let (data, text) = if let Some(Kind::Blob { bytes, .. }) = blob {
                    (bytes, false)
                } else {
                    (vm.to_string(input)?.into_bytes(), true)
                };
                if data.len() > 1024 * 1024 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "socket message"),
                    ));
                }
                if let Some(Object {
                    kind: Kind::Socket { buffered, .. },
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    if buffered.saturating_add(data.len()) > 1024 * 1024 {
                        return Err(error(
                            vm,
                            ApiError::new("QuotaExceededError", "socket buffered data"),
                        ));
                    }
                    *buffered += data.len();
                }
                self.command(vm, Command::SocketSend { id, data, text })?;
                JsValue::Undefined
            }
            "socket:close" => {
                let code = if args.is_empty() {
                    1000
                } else {
                    vm.to_number(arg(args, 0))? as u16
                };
                if code != 1000 && !(3000..=4999).contains(&code) {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidAccessError", "socket close code"),
                    ));
                }
                let reason = if args.len() > 1 {
                    text(vm, args, 1)?
                } else {
                    String::new()
                };
                if reason.len() > 123 {
                    return Err(error(
                        vm,
                        ApiError::new("SyntaxError", "socket reason length"),
                    ));
                }
                if let Some(Object {
                    kind: Kind::Socket { ready, .. },
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    *ready = 2;
                }
                self.command(vm, Command::SocketClose { id, code, reason })?;
                JsValue::Undefined
            }
            "EventSource" => {
                let input = text(vm, args, 0)?;
                let url =
                    web_url(&self.state.borrow().dom.url(), &input).map_err(|e| error(vm, e))?;
                let credentials = get(vm, arg(args, 1), "withCredentials")?.truthy();
                let document = self.state.borrow().dom.url();
                let policy = Csp {
                    inner: self.state.borrow().dom.policy(),
                };
                let id = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::EventSource {
                        ready: 0,
                        url: url.clone(),
                        credentials,
                    })
                    .map_err(|e| error(vm, e))?;
                self.state.borrow_mut().pending.insert(id);
                self.command(
                    vm,
                    Command::EventSource {
                        id,
                        url,
                        credentials,
                        document,
                        policy,
                    },
                )?;
                wrap(vm, id, "EventSource")?
            }
            "sse:close" => {
                if let Some(Object {
                    kind: Kind::EventSource { ready, .. },
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    *ready = 2;
                }
                self.state.borrow_mut().pending.remove(&id);
                self.command(vm, Command::Abort { id })?;
                JsValue::Undefined
            }
            "geo:getCurrentPosition" => {
                let success = arg(args, 0);
                let failure = arg(args, 1);
                if !vm.callable(&success) {
                    return Err(error(
                        vm,
                        ApiError::new("TypeError", "geolocation callback"),
                    ));
                }
                let id = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::GeoRequest { success, failure })
                    .map_err(|e| error(vm, e))?;
                let (profile, url, gesture) = {
                    let s = self.state.borrow();
                    (s.config.profile.clone(), s.dom.url(), s.gesture)
                };
                let decision = if !secure_context(&url) {
                    Some("denied".into())
                } else if self.state.borrow().grants.contains("geolocation") {
                    Some("granted".into())
                } else {
                    profile.permission(&url, "geolocation")
                };
                self.state.borrow_mut().pending.insert(id);
                if let Some(decision) = decision {
                    self.state.borrow_mut().jobs.push_back(Job::Permission {
                        id,
                        name: "geolocation".into(),
                        decision,
                    });
                } else {
                    self.command(
                        vm,
                        Command::Permission {
                            id,
                            name: "geolocation".into(),
                            gesture,
                        },
                    )?;
                }
                JsValue::Undefined
            }
            "storageManager:persist" => {
                if !self.state.borrow().config.profile.is_persistent() {
                    return self.ready(vm, Ok(JsValue::Bool(false))).map(Some);
                }
                let promise = self.permission(vm, "persistent-storage")?;
                if let Some(id) = vm.host_id(&promise) {
                    let mut state = self.state.borrow_mut();
                    if let Some(Object {
                        kind:
                            Kind::Promise(Promise {
                                result: Some((_, value)),
                                ..
                            }),
                        ..
                    }) = state.objects.get_mut(&id)
                    {
                        *value = JsValue::Bool(*value == JsValue::string("granted"));
                    } else {
                        state.permission_bools.insert(id);
                    }
                }
                promise
            }
            "Notification" => {
                let title = text(vm, args, 0)?;
                let body = get(vm, arg(args, 1), "body")?;
                let body = if body == JsValue::Undefined {
                    String::new()
                } else {
                    vm.to_string(body)?
                };
                let allowed = {
                    let s = self.state.borrow();
                    s.grants.contains("notifications")
                        || s.config
                            .profile
                            .permission(&s.dom.url(), "notifications")
                            .as_deref()
                            == Some("granted")
                };
                if !allowed {
                    return Err(error(
                        vm,
                        ApiError::new("NotAllowedError", "notification permission required"),
                    ));
                }
                if self.state.borrow().config.platform.is_none() {
                    return Err(error(
                        vm,
                        ApiError::new("NotSupportedError", "notification backend unavailable"),
                    ));
                }
                self.command(vm, Command::Notify { title, body })?;
                vm.object()?
            }
            _ if name.starts_with("params:") => {
                let key = if matches!(name, "params:sort" | "params:toString" | "params:entries") {
                    String::new()
                } else {
                    text(vm, args, 0)?
                };
                let value = if matches!(name, "params:set" | "params:append") {
                    Some(text(vm, args, 1)?)
                } else {
                    None
                };
                let Some((mut values, owner)) = self.state.borrow().params(id) else {
                    return Err(error(
                        vm,
                        ApiError::new("TypeError", "URLSearchParams receiver"),
                    ));
                };
                match name {
                    "params:get" => {
                        return Ok(Some(
                            values
                                .iter()
                                .find(|(k, _)| k == &key)
                                .map(|(_, v)| JsValue::string(v))
                                .unwrap_or(JsValue::Null),
                        ))
                    }
                    "params:has" => {
                        return Ok(Some(JsValue::Bool(values.iter().any(|(k, _)| k == &key))))
                    }
                    "params:getAll" => {
                        return Ok(Some(
                            vm.array(
                                values
                                    .iter()
                                    .filter(|(k, _)| k == &key)
                                    .map(|(_, v)| JsValue::string(v))
                                    .collect(),
                            )?,
                        ))
                    }
                    "params:toString" => {
                        return Ok(Some(JsValue::string(
                            url::form_urlencoded::Serializer::new(String::new())
                                .extend_pairs(&values)
                                .finish(),
                        )))
                    }
                    "params:entries" => {
                        let mut list = Vec::new();
                        for (k, v) in values {
                            list.push(vm.array(vec![JsValue::string(k), JsValue::string(v)])?);
                        }
                        return Ok(Some(vm.array(list)?));
                    }
                    "params:delete" => values.retain(|(k, _)| k != &key),
                    "params:sort" => values.sort_by(|a, b| a.0.cmp(&b.0)),
                    "params:append" => values.push((key, value.unwrap_or_default())),
                    "params:set" => {
                        let first = values
                            .iter()
                            .position(|(k, _)| k == &key)
                            .unwrap_or(values.len());
                        values.retain(|(k, _)| k != &key);
                        values.insert(first.min(values.len()), (key, value.unwrap_or_default()));
                    }
                    _ => return Ok(None),
                }
                let encoded = url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(&values)
                    .finish();
                vm.check_string_bytes(encoded.len())?;
                let mut s = self.state.borrow_mut();
                if let Some(Object {
                    kind: Kind::Params { values: v, .. },
                    ..
                }) = s.objects.get_mut(&id)
                {
                    *v = values;
                }
                if let Some(owner) = owner {
                    if let Some(Object {
                        kind: Kind::Url(url),
                        ..
                    }) = s.objects.get_mut(&owner)
                    {
                        url.set_query(if encoded.is_empty() {
                            None
                        } else {
                            Some(&encoded)
                        });
                    }
                }
                JsValue::Undefined
            }
            _ if name.starts_with("form:") => {
                let key = text(vm, args, 0)?;
                let input = arg(args, 1);
                let mut blob = if matches!(name, "form:append" | "form:set") {
                    vm.host_id(&input)
                        .and_then(|i| self.state.borrow().objects.get(&i).map(|o| o.kind.clone()))
                } else {
                    None
                };
                let value = if let Some(Kind::Blob { bytes, mime }) = blob.take() {
                    let name = if args.len() > 2 {
                        text(vm, args, 2)?
                    } else {
                        "blob".into()
                    };
                    Some(FormValue::Blob { bytes, mime, name })
                } else if matches!(name, "form:append" | "form:set") {
                    Some(FormValue::Text(vm.to_string(input)?))
                } else {
                    None
                };
                let Some(Kind::FormData(mut values)) = snapshot() else {
                    return Err(error(vm, ApiError::new("TypeError", "FormData receiver")));
                };
                if matches!(name, "form:get" | "form:getAll") {
                    let mut list = Vec::new();
                    for (_, v) in values.iter().filter(|(k, _)| k == &key) {
                        list.push(match v {
                            FormValue::Text(t) => JsValue::string(t),
                            FormValue::Blob { bytes, mime, name } => {
                                let v = self.allocate(
                                    vm,
                                    Kind::Blob {
                                        bytes: bytes.clone(),
                                        mime: mime.clone(),
                                    },
                                    "File",
                                )?;
                                vm.set(v.clone(), "name", JsValue::string(name))?;
                                v
                            }
                        });
                    }
                    return Ok(Some(if name == "form:get" {
                        list.into_iter().next().unwrap_or(JsValue::Null)
                    } else {
                        vm.array(list)?
                    }));
                }
                if name == "form:has" {
                    return Ok(Some(JsValue::Bool(values.iter().any(|(k, _)| k == &key))));
                }
                if matches!(name, "form:delete" | "form:set") {
                    values.retain(|(k, _)| k != &key);
                }
                if let Some(v) = value {
                    values.push((key, v));
                }
                if values.len() > 10_000 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "form entry count"),
                    ));
                }
                if let Some(Object {
                    kind: Kind::FormData(v),
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    *v = values;
                }
                JsValue::Undefined
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
}
