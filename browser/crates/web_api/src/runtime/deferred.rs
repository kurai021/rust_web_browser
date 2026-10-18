use super::*;
impl State {
    pub(super) fn promise(&mut self) -> Result<u64> {
        self.allocate(Kind::Promise(Promise {
            native: JsValue::Undefined,
            result: None,
        }))
    }
    pub(super) fn settle(&mut self, id: u64, ok: bool, value: JsValue) {
        let Some(Object {
            kind: Kind::Promise(p),
            ..
        }) = self.objects.get_mut(&id)
        else {
            return;
        };
        if p.result.is_some() {
            return;
        }
        p.result = Some((ok, value.clone()));
        self.jobs.push_back(Job::Settle {
            id,
            ok,
            value: value.clone(),
        });
    }
}
impl PageRuntime {
    pub(super) fn drain_jobs(&mut self) -> std::result::Result<(), JsError> {
        if self.dom.is_stopped() {
            self.state.borrow_mut().jobs.clear();
            return Ok(());
        }
        for _ in 0..2048 {
            self.dom.vm.drain_microtasks()?;
            for error in self.dom.vm.take_async_errors() {
                self.state.borrow().dom.console(error.to_string());
            }
            let notifications = self.dom.vm.take_promise_notifications();
            if !notifications.is_empty() {
                for notification in notifications {
                    let data = self.dom.vm.object()?;
                    self.dom
                        .vm
                        .set(data.clone(), "promise", notification.promise)?;
                    self.dom
                        .vm
                        .set(data.clone(), "reason", notification.reason.clone())?;
                    let allowed = self.dom.dispatch_detail_in_task(
                        0,
                        if notification.handled {
                            "rejectionhandled"
                        } else {
                            "unhandledrejection"
                        },
                        data,
                    )?;
                    if allowed && !notification.handled {
                        let reason = self
                            .dom
                            .vm
                            .to_string(notification.reason)
                            .unwrap_or_else(|_| "<unprintable rejection>".into());
                        self.state
                            .borrow()
                            .dom
                            .console(format!("Unhandled promise rejection: {reason}"));
                    }
                }
                continue;
            }
            let job = self.state.borrow_mut().jobs.pop_front();
            let Some(job) = job else {
                if self.observer_checkpoint()? {
                    continue;
                }
                if self.dom.vm.has_microtasks() {
                    continue;
                }
                self.flush_canvases()?;
                return Ok(());
            };
            if !matches!(job, Job::Settle { .. }) {
                self.dom.vm.begin_task();
            }
            match job {
                Job::Settle { id, ok, value } => {
                    let native = WebHost {
                        state: self.state.clone(),
                    }
                    .promise_value(id);
                    if ok {
                        self.dom.vm.resolve_promise(native, value)?;
                    } else {
                        self.dom.vm.reject_promise(native, value)?;
                    }
                }
                Job::Observer { id } => self.observer_callback(id)?,
                Job::Database(task) => self.database_task(task)?,
                Job::Permission { id, name, decision } => {
                    self.accept(Completion::Permission { id, name, decision })?
                }
                Job::ReaderComplete { id, value } => {
                    if let Some(Object {
                        kind: Kind::FileReader { ready, result, .. },
                        ..
                    }) = self.state.borrow_mut().objects.get_mut(&id)
                    {
                        *ready = 2;
                        *result = value;
                    }
                    self.deliver_event(id, "load", JsValue::Undefined)?;
                    self.deliver_event(id, "loadend", JsValue::Undefined)?;
                }
                Job::Callback {
                    function,
                    this,
                    args,
                } => {
                    if let Err(e) = self.dom.call_in_task(function, this, args) {
                        if e.is_resource_limit() {
                            return Err(e);
                        }
                        self.state.borrow().dom.console(e.to_string());
                    }
                }
                Job::Event { target, name, data } => self.deliver_event(target, &name, data)?,
                Job::DomEvent { name, data } => {
                    self.dom.dispatch_detail_in_task(0, &name, data)?;
                }
                Job::XhrEvent {
                    id,
                    operation,
                    name,
                    data,
                } => {
                    if self.current_xhr(id, operation) {
                        self.deliver_event(id, &name, data)?;
                    }
                }
                Job::XhrState {
                    id,
                    operation,
                    ready,
                } => {
                    if !self.current_xhr(id, operation) {
                        continue;
                    }
                    if let Some(Object {
                        kind: Kind::Xhr(xhr),
                        ..
                    }) = self.state.borrow_mut().objects.get_mut(&id)
                    {
                        xhr.ready = ready;
                    }
                    self.deliver_event(id, "readystatechange", JsValue::Undefined)?;
                }
            }
        }
        self.dom
            .stop(JsError::new(ErrorKind::Timeout, "web job queue budget"))
    }
    pub(super) fn deliver_event(
        &mut self,
        id: u64,
        name: &str,
        data: JsValue,
    ) -> std::result::Result<(), JsError> {
        self.web_event(id, name, data).map(|_| ())
    }
    pub(super) fn web_event(
        &mut self,
        id: u64,
        name: &str,
        data: JsValue,
    ) -> std::result::Result<(bool, bool), JsError> {
        let (callbacks, once) = {
            let state = self.state.borrow();
            let Some(object) = state.objects.get(&id) else {
                return Ok((true, false));
            };
            let mut callbacks = Vec::new();
            if let Some(callback) = object.properties.get(&format!("on{name}")) {
                if self.dom.vm.callable(callback) {
                    callbacks.push(callback.clone());
                }
            }
            let mut once = Vec::new();
            for (event, callback, is_once) in &object.listeners {
                if event == name {
                    callbacks.push(callback.clone());
                    if *is_once {
                        once.push(callback.clone());
                    }
                }
            }
            (callbacks, once)
        };
        if let Some(object) = self.state.borrow_mut().objects.get_mut(&id) {
            object
                .listeners
                .retain(|(event, callback, _)| event != name || !once.contains(callback));
        }
        let target = wrap(&mut self.dom.vm, id, "EventTarget")?;
        let event = self.dom.vm.object()?;
        self.dom
            .vm
            .set(event.clone(), "type", JsValue::string(name))?;
        self.dom.vm.set(event.clone(), "target", target.clone())?;
        self.dom
            .vm
            .set(event.clone(), "currentTarget", target.clone())?;
        self.dom.vm.set(event.clone(), "data", data.clone())?;
        let prevent = self.state.borrow().function("event:preventDefault");
        self.dom.vm.set(event.clone(), "preventDefault", prevent)?;
        self.dom
            .vm
            .set(event.clone(), "defaultPrevented", JsValue::Bool(false))?;
        if self
            .state
            .borrow()
            .objects
            .get(&id)
            .is_some_and(|o| matches!(o.kind, Kind::EventSource { .. }))
        {
            if let JsValue::Object(_) = &data {
                let text = self.dom.vm.get(data.clone(), "__sseData")?;
                if text != JsValue::Undefined {
                    self.dom.vm.set(event.clone(), "data", text)?;
                }
            }
        }
        if let JsValue::Object(_) = data {
            for key in [
                "code",
                "reason",
                "lastEventId",
                "error",
                "oldVersion",
                "newVersion",
                "loaded",
                "total",
                "lengthComputable",
                "message",
            ] {
                let value = self.dom.vm.get(data.clone(), key)?;
                if value != JsValue::Undefined {
                    self.dom.vm.set(event.clone(), key, value)?;
                }
            }
        }
        let mut succeeded = true;
        for callback in callbacks {
            if let Err(e) = self
                .dom
                .call_in_task(callback, target.clone(), vec![event.clone()])
            {
                if e.is_resource_limit() {
                    return Err(e);
                }
                self.state.borrow().dom.console(e.to_string());
                succeeded = false;
            }
        }
        let prevented = self.dom.vm.get(event, "defaultPrevented")?.truthy();
        Ok((succeeded, prevented))
    }
    pub(super) fn accept(&mut self, completion: Completion) -> std::result::Result<(), JsError> {
        let active = match &completion {
            Completion::Text { id, .. }
            | Completion::Binary { id, .. }
            | Completion::StreamRetry { id }
            | Completion::Message { id, .. }
            | Completion::Opened { id, .. }
            | Completion::ServerEvent { id, .. }
            | Completion::Closed { id, .. }
            | Completion::Fetch { id, .. } => Some(*id),
            _ => None,
        };
        if active.is_some_and(|id| !self.state.borrow().pending.contains(&id)) {
            return Ok(());
        }
        match completion {
            Completion::Module { id, result } => {
                if !self.state.borrow_mut().pending.remove(&id) {
                    return Ok(());
                }
                let result = result
                    .map_err(|e| error(&mut self.dom.vm, e))
                    .and_then(|graph| self.install_module_graph(&graph));
                let (ok, value) = match result {
                    Ok(value) => (true, value),
                    Err(error) if error.is_resource_limit() => return Err(error),
                    Err(error) => (
                        false,
                        if let Some(value) = error.thrown {
                            value
                        } else {
                            self.dom.vm.error_value(error)?
                        },
                    ),
                };
                self.state.borrow_mut().settle(id, ok, value);
            }
            Completion::WorkerClosed { id } => {
                self.state.borrow_mut().pending.remove(&id);
                if let Some(Object {
                    kind: Kind::Worker { alive },
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    *alive = false;
                }
            }
            Completion::Text { id, text } => {
                self.dom.vm.check_string_bytes(text.len())?;
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "message".into(),
                    data: JsValue::string(text),
                });
            }
            Completion::Binary { id, bytes } => {
                let blob = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::Blob {
                        bytes,
                        mime: String::new(),
                    })
                    .map_err(|e| error(&mut self.dom.vm, e))?;
                let data = wrap(&mut self.dom.vm, blob, "Blob")?;
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "message".into(),
                    data,
                });
            }
            Completion::StreamRetry { id } => {
                if let Some(Object {
                    kind: Kind::EventSource { ready, .. },
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    *ready = 0;
                }
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "error".into(),
                    data: JsValue::Undefined,
                });
            }
            Completion::Sent { id, bytes } => {
                if let Some(Object {
                    kind: Kind::Socket { buffered, .. },
                    ..
                }) = self.state.borrow_mut().objects.get_mut(&id)
                {
                    *buffered = buffered.saturating_sub(bytes);
                }
            }
            Completion::Fetch { id, result } => {
                self.state.borrow_mut().pending.remove(&id);
                let target = match self.state.borrow().objects.get(&id).map(|o| &o.kind) {
                    Some(Kind::XhrOperation { target }) => Some(*target),
                    _ => None,
                };
                if let Some(target) = target {
                    let operation = id;
                    let id = target;
                    if !self.current_xhr(id, operation) {
                        return Ok(());
                    }
                    let mut event_name = "error";
                    let mut loaded = 0usize;
                    let mut total = 0u64;
                    let mut computable = false;
                    let result = match result {
                        Ok(response) => {
                            loaded = response.body.len();
                            if let Some(value) = response
                                .headers
                                .get("content-length")
                                .and_then(|s| s.parse::<u64>().ok())
                            {
                                total = value;
                                computable = true;
                            }
                            if let Some(Object {
                                kind: Kind::Xhr(xhr),
                                ..
                            }) = self.state.borrow_mut().objects.get_mut(&id)
                            {
                                if xhr.aborted {
                                    return Ok(());
                                }
                                xhr.response = Some(response);
                                xhr.sent = false;
                            }
                            true
                        }
                        Err(e) => {
                            if e.name == "TimeoutError" {
                                event_name = "timeout";
                            }
                            self.state.borrow().dom.console(e.to_string());
                            false
                        }
                    };
                    let progress = self.dom.vm.object()?;
                    for (name, value) in [
                        ("loaded", JsValue::Number(loaded as f64)),
                        ("total", JsValue::Number(total as f64)),
                        ("lengthComputable", JsValue::Bool(computable)),
                    ] {
                        self.dom.vm.set(progress.clone(), name, value)?;
                    }
                    let mut state = self.state.borrow_mut();
                    if result {
                        for ready in [2, 3] {
                            state.jobs.push_back(Job::XhrState {
                                id,
                                operation,
                                ready,
                            });
                        }
                        state.jobs.push_back(Job::XhrEvent {
                            id,
                            operation,
                            name: "progress".into(),
                            data: progress.clone(),
                        });
                        state.jobs.push_back(Job::XhrState {
                            id,
                            operation,
                            ready: 4,
                        });
                        state.jobs.push_back(Job::XhrEvent {
                            id,
                            operation,
                            name: "load".into(),
                            data: progress.clone(),
                        });
                    } else {
                        if let Some(Object {
                            kind: Kind::Xhr(x), ..
                        }) = state.objects.get_mut(&id)
                        {
                            x.sent = false;
                            x.response = None;
                        }
                        state.jobs.push_back(Job::XhrState {
                            id,
                            operation,
                            ready: 4,
                        });
                        state.jobs.push_back(Job::XhrEvent {
                            id,
                            operation,
                            name: event_name.into(),
                            data: progress.clone(),
                        });
                    }
                    state.jobs.push_back(Job::XhrEvent {
                        id,
                        operation,
                        name: "loadend".into(),
                        data: progress,
                    });
                } else {
                    match result {
                        Ok(response) => {
                            let value = WebHost {
                                state: self.state.clone(),
                            }
                            .allocate(
                                &mut self.dom.vm,
                                Kind::Response {
                                    response,
                                    used: false,
                                },
                                "Response",
                            )?;
                            self.state.borrow_mut().settle(id, true, value);
                        }
                        Err(e) => {
                            let e = error(&mut self.dom.vm, e);
                            let value = if let Some(v) = e.thrown {
                                v
                            } else {
                                self.dom.vm.error_value(e)?
                            };
                            self.state.borrow_mut().settle(id, false, value);
                        }
                    }
                }
            }
            Completion::Permission { id, name, decision } => {
                self.state.borrow_mut().pending.remove(&id);
                self.record_permission(&name, &decision)
                    .map_err(|e| error(&mut self.dom.vm, e))?;
                let granted = matches!(decision.as_str(), "once" | "always" | "granted");
                if name == "fullscreen" {
                    let node = self.state.borrow_mut().fullscreen_requests.remove(&id);
                    if granted {
                        if let Some(node) = node {
                            let mut s = self.state.borrow_mut();
                            s.fullscreen = Some(node);
                            s.enqueue(Command::Fullscreen { enter: true })
                                .map_err(|e| error(&mut self.dom.vm, e))?;
                            s.jobs.push_back(Job::DomEvent {
                                name: "fullscreenchange".into(),
                                data: JsValue::Undefined,
                            });
                            s.settle(id, true, JsValue::Undefined);
                        }
                    } else {
                        let value = error(
                            &mut self.dom.vm,
                            ApiError::new("NotAllowedError", "fullscreen permission denied"),
                        )
                        .thrown
                        .unwrap_or(JsValue::Undefined);
                        self.state.borrow_mut().settle(id, false, value);
                    }
                    return Ok(());
                }
                let geo = self
                    .state
                    .borrow()
                    .objects
                    .get(&id)
                    .and_then(|o| match &o.kind {
                        Kind::GeoRequest { failure, .. } => Some(failure.clone()),
                        _ => None,
                    });
                if let Some(failure) = geo {
                    let value = self.dom.vm.object()?;
                    self.dom.vm.set(
                        value.clone(),
                        "code",
                        JsValue::Number(if granted { 2.0 } else { 1.0 }),
                    )?;
                    self.dom.vm.set(
                        value.clone(),
                        "message",
                        JsValue::string(if granted {
                            "No local geolocation provider"
                        } else {
                            "Geolocation permission denied"
                        }),
                    )?;
                    if self.dom.vm.callable(&failure) {
                        self.state.borrow_mut().jobs.push_back(Job::Callback {
                            function: failure,
                            this: JsValue::Undefined,
                            args: vec![value],
                        });
                    }
                    return Ok(());
                }
                if name == "notifications" {
                    let ctor = self.state.borrow().function("Notification");
                    self.dom.vm.set(
                        ctor,
                        "permission",
                        JsValue::string(if granted { "granted" } else { "denied" }),
                    )?;
                }
                let boolean = self.state.borrow_mut().permission_bools.remove(&id);
                self.state.borrow_mut().settle(
                    id,
                    true,
                    if boolean {
                        JsValue::Bool(granted)
                    } else {
                        JsValue::string(if granted { "granted" } else { "denied" })
                    },
                );
            }
            Completion::Clipboard { id, result } => {
                self.state.borrow_mut().pending.remove(&id);
                match result {
                    Ok(text) => self
                        .state
                        .borrow_mut()
                        .settle(id, true, JsValue::string(text)),
                    Err(e) => {
                        let e = error(&mut self.dom.vm, e);
                        let v = e.thrown.unwrap_or(JsValue::Undefined);
                        self.state.borrow_mut().settle(id, false, v);
                    }
                }
            }
            Completion::Message { id, data } => {
                let value = self.dom.vm.deserialize_clone(&data)?;
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "message".into(),
                    data: value,
                });
            }
            Completion::Opened { id, protocol } => {
                if let Some(object) = self.state.borrow_mut().objects.get_mut(&id) {
                    match &mut object.kind {
                        Kind::Socket {
                            ready, protocol: p, ..
                        } => {
                            *ready = 1;
                            *p = protocol;
                        }
                        Kind::EventSource { ready, .. } => *ready = 1,
                        _ => {}
                    }
                }
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "open".into(),
                    data: JsValue::Undefined,
                });
            }
            Completion::Closed { id, code, reason } => {
                self.state.borrow_mut().pending.remove(&id);
                if let Some(object) = self.state.borrow_mut().objects.get_mut(&id) {
                    if let Kind::Socket { ready, .. } = &mut object.kind {
                        *ready = 3;
                    }
                    if let Kind::EventSource { ready, .. } = &mut object.kind {
                        *ready = 2;
                        return Ok(());
                    }
                }
                let data = self.dom.vm.object()?;
                self.dom
                    .vm
                    .set(data.clone(), "code", JsValue::Number(code as f64))?;
                self.dom
                    .vm
                    .set(data.clone(), "reason", JsValue::string(reason))?;
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "close".into(),
                    data,
                });
            }
            Completion::Error { id, error: e } => {
                let value = error(&mut self.dom.vm, e)
                    .thrown
                    .unwrap_or(JsValue::Undefined);
                self.state.borrow_mut().pending.remove(&id);
                if let Some(object) = self.state.borrow_mut().objects.get_mut(&id) {
                    match &mut object.kind {
                        Kind::Worker { alive } => *alive = false,
                        Kind::Socket { ready, .. } => *ready = 3,
                        Kind::EventSource { ready, .. } => *ready = 2,
                        _ => {}
                    }
                }
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: "error".into(),
                    data: value,
                });
            }
            Completion::ServerEvent {
                id,
                event,
                data,
                last_id,
            } => {
                let value = self.dom.vm.object()?;
                self.dom
                    .vm
                    .set(value.clone(), "lastEventId", JsValue::string(last_id))?;
                self.dom
                    .vm
                    .set(value.clone(), "__sseData", JsValue::string(data))?;
                self.state.borrow_mut().jobs.push_back(Job::Event {
                    target: id,
                    name: event,
                    data: value,
                });
            }
        }
        Ok(())
    }
    fn current_xhr(&self, id: u64, operation: u64) -> bool {
        matches!(self.state.borrow().objects.get(&id).map(|o|&o.kind),Some(Kind::Xhr(x))if x.operation==Some(operation)&&!x.aborted)
    }
    pub fn install_module_graph(
        &mut self,
        graph: &crate::modules::ModuleGraph,
    ) -> std::result::Result<JsValue, JsError> {
        for source in &graph.sources {
            self.dom.vm.define_module(&source.url, &source.source)?;
            self.dom.vm.set_module_base(&source.url, &source.base);
        }
        for (alias, canonical) in &graph.aliases {
            self.dom.vm.alias_module(alias, canonical)?;
        }
        self.dom.vm.evaluate_module_in_task(&graph.entry)
    }
}
