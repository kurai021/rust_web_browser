//! Rust DOM trampolines. Callbacks run only after the host borrow is restored.
use crate::*;
use html::{Namespace, NodeData};
use js::{ErrorKind, HostResult};

pub(crate) struct DomHost {
    pub state: Rc<RefCell<State>>,
}
fn error(message: impl Into<String>) -> JsError {
    JsError::new(ErrorKind::TypeError, message)
}
fn dom(error: html::DomError) -> JsError {
    JsError::new(ErrorKind::Error, error.to_string())
}
fn arg(args: &[JsValue], i: usize) -> JsValue {
    args.get(i).cloned().unwrap_or(JsValue::Undefined)
}
fn string(vm: &mut Vm, args: &[JsValue], i: usize) -> Result<String, JsError> {
    vm.to_string(arg(args, i))
}
fn wrap(vm: &mut Vm, document: &Document, node: Option<NodeId>) -> Result<JsValue, JsError> {
    if let Some(node) = node {
        let class = match document.try_get(node).map(|n| &n.data) {
            Some(NodeData::Document) => "HTMLDocument",
            Some(NodeData::Element(_)) => "HTMLElement",
            Some(NodeData::Text(_)) => "Text",
            Some(NodeData::Comment(_)) => "Comment",
            _ => "DocumentFragment",
        };
        vm.host_object(node_host(node, 0), class)
    } else {
        Ok(JsValue::Null)
    }
}
pub(crate) fn install(vm: &mut Vm, state: &Rc<RefCell<State>>) -> Result<(), JsError> {
    vm.set_global_host(WINDOW, "Window")?;
    let document = wrap(vm, &state.borrow().document, Some(0))?;
    vm.set_global("document", document)?;
    let window = vm.global_value();
    for name in ["window", "self", "globalThis"] {
        vm.set_global(name, window.clone())?;
    }
    let methods = [
        "createElement",
        "createTextNode",
        "createDocumentFragment",
        "getElementById",
        "getElementsByTagName",
        "getElementsByClassName",
        "querySelector",
        "querySelectorAll",
        "getAttribute",
        "setAttribute",
        "removeAttribute",
        "hasAttribute",
        "matches",
        "closest",
        "appendChild",
        "removeChild",
        "insertBefore",
        "replaceChild",
        "cloneNode",
        "contains",
        "insertAdjacentHTML",
        "append",
        "prepend",
        "remove",
        "addEventListener",
        "removeEventListener",
        "dispatchEvent",
        "click",
        "focus",
        "blur",
        "submit",
        "reset",
        "checkValidity",
        "reportValidity",
        "setCustomValidity",
        "toggle",
        "add",
        "removeClass",
        "containsClass",
        "replaceClass",
        "item",
        "setProperty",
        "getPropertyValue",
        "removeProperty",
        "preventDefault",
        "stopPropagation",
        "stopImmediatePropagation",
        "initEvent",
    ];
    for name in methods {
        let function = vm.native(&format!("host:{name}"))?;
        state.borrow_mut().functions.insert(name.into(), function);
    }
    for name in [
        "setTimeout",
        "setInterval",
        "clearTimeout",
        "clearInterval",
        "requestAnimationFrame",
        "cancelAnimationFrame",
        "getComputedStyle",
        "matchMedia",
        "Event",
        "CustomEvent",
        "MouseEvent",
        "KeyboardEvent",
        "InputEvent",
        "SubmitEvent",
        "FocusEvent",
        "EventTarget",
        "Node",
        "Element",
        "Document",
        "HTMLElement",
    ] {
        let function = vm.native(&format!("host:{name}"))?;
        if name == "Event" {
            for (key, value) in [
                ("NONE", 0.0),
                ("CAPTURING_PHASE", 1.0),
                ("AT_TARGET", 2.0),
                ("BUBBLING_PHASE", 3.0),
            ] {
                vm.set(function.clone(), key, JsValue::Number(value))?;
            }
        }
        vm.set_global(name, function)?;
    }
    Ok(())
}
impl DomHost {
    fn function(&self, name: &str) -> Option<JsValue> {
        self.state.borrow().functions.get(name).cloned()
    }
    fn query(
        &self,
        vm: &mut Vm,
        node: NodeId,
        selector: &str,
        include_root: bool,
    ) -> Result<Vec<NodeId>, JsError> {
        let selectors = css::selectors::parse_selector_list(selector)
            .map_err(|e| JsError::new(ErrorKind::SyntaxError, e.to_string()))?;
        let state = self.state.borrow();
        let mut matches = Vec::new();
        for id in state.document.elements_from(node) {
            vm.checkpoint()?;
            if (include_root || id != node)
                && selectors.iter().any(|s| {
                    s.matches(
                        &state.document,
                        id,
                        css::selectors::MatchContext {
                            focused: state.focused,
                            ..Default::default()
                        },
                    )
                })
            {
                matches.push(id);
            }
        }
        Ok(matches)
    }
    fn dispatch_plan(
        &mut self,
        vm: &mut Vm,
        target: NodeId,
        event: u64,
    ) -> Result<HostResult, JsError> {
        let mut state = self.state.borrow_mut();
        let Some(data) = state.events.get_mut(&event) else {
            return Err(error("unknown Event"));
        };
        data.target = target;
        if data.phase != 0 {
            return Err(error("InvalidStateError: event is already dispatching"));
        }
        data.stopped = false;
        data.immediate = false;
        data.phase = 1;
        let name = data.name.clone();
        let bubbles = data.bubbles;
        let mut path = vec![target];
        let mut current = state.document.try_get(target).and_then(|n| n.parent);
        while let Some(node) = current {
            if path.len() > 512 {
                return Err(error("event path limit"));
            }
            path.push(node);
            current = state.document.try_get(node).and_then(|n| n.parent);
        }
        let mut queue = Vec::new();
        for &node in path.iter().rev() {
            for listener in &state.listeners {
                if listener.node == node && listener.event == name && listener.capture {
                    queue.push(listener.clone());
                }
            }
        }
        for &node in &path {
            if node != target && !bubbles {
                break;
            }
            for listener in &state.listeners {
                if listener.node == node && listener.event == name && !listener.capture {
                    queue.push(listener.clone());
                }
            }
            if state.policy.inline_allowed("", None) {
                if let Some(source) = state.document.get_attribute(node, &format!("on{name}")) {
                    let function = vm.compile_event_handler(source)?;
                    queue.push(Listener {
                        node,
                        event: name.clone(),
                        callback: function,
                        capture: false,
                        once: false,
                        passive: false,
                        inline: true,
                    });
                }
            }
        }
        if name == "click"
            && state.document.is_element_named(target, "input")
            && state.document.get_attribute(target, "type") == Some("checkbox")
        {
            let old = state.document.control_checked(target);
            state.document.set_control_checked(target, !old);
            state.changed();
            if let Some(data) = state.events.get_mut(&event) {
                data.old_checked = Some(old);
            }
        }
        if let Some(data) = state.events.get_mut(&event) {
            data.queue = queue.iter().cloned().collect();
        }
        let mut calls = Vec::new();
        for listener in queue {
            let this = wrap(vm, &state.document, Some(listener.node))?;
            let event = vm.host_object(event, "Event")?;
            calls.push((listener.callback, this, vec![event]));
        }
        Ok(HostResult {
            value: JsValue::Bool(true),
            calls,
            finish: Some(event),
            catch_callback_errors: true,
        })
    }
}
impl HostHooks for DomHost {
    fn retained_bytes(&self) -> usize {
        let state = self.state.borrow();
        let mut bytes = state
            .document
            .retained_bytes()
            .saturating_add(state.listeners.len().saturating_mul(192))
            .saturating_add(state.events.len().saturating_mul(256));
        for timer in state.timers.values() {
            bytes = bytes
                .saturating_add(128)
                .saturating_add(timer.args.len().saturating_mul(32));
            for value in std::iter::once(&timer.callback).chain(&timer.args) {
                if let JsValue::String(s) = value {
                    bytes = bytes.saturating_add(s.len());
                }
            }
        }
        for line in &state.console {
            bytes = bytes.saturating_add(line.capacity());
        }
        bytes
    }
    fn print(&mut self, s: &str) {
        let mut state = self.state.borrow_mut();
        if state.console.len() < 1000 {
            state.console.push(s.chars().take(16_384).collect());
            state.changed();
        }
    }
    fn allow_dynamic_code(&self) -> bool {
        self.state.borrow().policy.eval_allowed()
    }
    fn get(&mut self, vm: &mut Vm, id: u64, key: &str) -> Result<Option<JsValue>, JsError> {
        if id == WINDOW {
            return Ok(match key {
                "document" => Some(wrap(vm, &self.state.borrow().document, Some(0))?),
                "innerWidth" => Some(JsValue::Number(
                    self.state.borrow().environment.width as f64,
                )),
                "innerHeight" => Some(JsValue::Number(
                    self.state.borrow().environment.height as f64,
                )),
                "devicePixelRatio" => Some(JsValue::Number(1.0)),
                "addEventListener" | "removeEventListener" | "dispatchEvent" => self.function(key),
                _ => None,
            });
        }
        if id >= EVENT_BASE {
            let state = self.state.borrow();
            let Some(event) = state.events.get(&id) else {
                return Ok(None);
            };
            return Ok(match key {
                "type" => Some(JsValue::string(&event.name)),
                "target" => Some(wrap(vm, &state.document, Some(event.target))?),
                "currentTarget" => Some(if event.phase == 0 {
                    JsValue::Null
                } else {
                    wrap(vm, &state.document, Some(event.current))?
                }),
                "eventPhase" => Some(JsValue::Number(event.phase as f64)),
                "bubbles" => Some(JsValue::Bool(event.bubbles)),
                "cancelable" => Some(JsValue::Bool(event.cancelable)),
                "defaultPrevented" => Some(JsValue::Bool(event.prevented)),
                "cancelBubble" => Some(JsValue::Bool(event.stopped)),
                "returnValue" => Some(JsValue::Bool(!event.prevented)),
                "isTrusted" => Some(JsValue::Bool(false)),
                "timeStamp" => Some(JsValue::Number(event.timestamp)),
                "detail" => Some(event.detail.clone()),
                "key" => Some(JsValue::string(&event.key)),
                "keyCode" | "which" => Some(JsValue::Number(match event.key.as_str() {
                    "Enter" => 13.0,
                    "Escape" => 27.0,
                    "Backspace" => 8.0,
                    "Tab" => 9.0,
                    other => other.chars().next().map_or(0.0, |c| c as u32 as f64),
                })),
                "preventDefault" | "stopPropagation" | "stopImmediatePropagation" | "initEvent" => {
                    state.functions.get(key).cloned()
                }
                _ => None,
            });
        }
        if id >= EVENT_TARGET_BASE {
            return Ok(match key {
                "addEventListener" | "removeEventListener" | "dispatchEvent" => self.function(key),
                _ => None,
            });
        }
        let node = node_id(id).ok_or_else(|| error("invalid host"))?;
        let kind = id % 8;
        if kind == 1 || kind == 3 {
            if matches!(key, "setProperty" | "getPropertyValue" | "removeProperty") {
                return Ok(self.function(key));
            }
            let mut state = self.state.borrow_mut();
            if kind == 3 {
                if state.computed.is_none() {
                    let cascade = css::Cascade {
                        environment: state.environment,
                        context: css::selectors::MatchContext {
                            focused: state.focused,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    state.computed = Some(cascade.compute(&state.document, &state.stylesheets));
                }
                return Ok(Some(JsValue::string(
                    state
                        .computed
                        .as_ref()
                        .and_then(|s| s.get(node))
                        .map_or_else(String::new, |s| s.get_property_value(&css_name(key))),
                )));
            }
            return Ok(Some(JsValue::string(style_property(
                &state.document,
                node,
                &css_name(key),
            ))));
        }
        if kind == 2 {
            return Ok(match key {
                "add" => self.function("add"),
                "remove" => self.function("removeClass"),
                "toggle" => self.function("toggle"),
                "contains" => self.function("containsClass"),
                "replace" => self.function("replaceClass"),
                "item" => self.function("item"),
                "value" => Some(JsValue::string(
                    self.state
                        .borrow()
                        .document
                        .get_attribute(node, "class")
                        .unwrap_or(""),
                )),
                "length" => Some(JsValue::Number(
                    self.state
                        .borrow()
                        .document
                        .get_attribute(node, "class")
                        .unwrap_or("")
                        .split_ascii_whitespace()
                        .count() as f64,
                )),
                _ => None,
            });
        }
        if let Some(function) = self.function(key) {
            return Ok(Some(function));
        }
        let state = self.state.borrow();
        let document = &state.document;
        let n = document
            .try_get(node)
            .ok_or_else(|| error("unknown node"))?;
        let value = match key {
            "nodeType" => JsValue::Number(match n.data {
                NodeData::Document => 9.0,
                NodeData::Element(_) => 1.0,
                NodeData::Text(_) => 3.0,
                NodeData::Comment(_) => 8.0,
                NodeData::DocumentFragment => 11.0,
                NodeData::DocumentType { .. } => 10.0,
            }),
            "nodeName" => JsValue::string(match &n.data {
                NodeData::Element(el) => el.tag_name.to_uppercase(),
                NodeData::Text(_) => "#text".into(),
                NodeData::Comment(_) => "#comment".into(),
                NodeData::DocumentFragment => "#document-fragment".into(),
                _ => "#document".into(),
            }),
            "tagName" => {
                if let NodeData::Element(el) = &n.data {
                    JsValue::string(el.tag_name.to_uppercase())
                } else {
                    JsValue::Undefined
                }
            }
            "parentNode" | "parentElement" => wrap(
                vm,
                document,
                n.parent.filter(|&p| {
                    key != "parentElement" || matches!(document.get(p).data, NodeData::Element(_))
                }),
            )?,
            "firstChild" => wrap(vm, document, n.children.first().copied())?,
            "lastChild" => wrap(vm, document, n.children.last().copied())?,
            "nextSibling" | "previousSibling" => {
                let sibling = n.parent.and_then(|p| {
                    let children = &document.get(p).children;
                    let i = children.iter().position(|&id| id == node)?;
                    if key == "nextSibling" {
                        children.get(i + 1).copied()
                    } else {
                        i.checked_sub(1).and_then(|i| children.get(i).copied())
                    }
                });
                wrap(vm, document, sibling)?
            }
            "childNodes" | "children" => {
                let mut values = Vec::new();
                for &child in &n.children {
                    if key == "children"
                        && !matches!(document.get(child).data, NodeData::Element(_))
                    {
                        continue;
                    }
                    values.push(wrap(vm, document, Some(child))?);
                }
                vm.array(values)?
            }
            "ownerDocument" => {
                if node == 0 {
                    JsValue::Null
                } else {
                    wrap(vm, document, Some(0))?
                }
            }
            "textContent" => {
                if node == 0 {
                    JsValue::Null
                } else {
                    JsValue::string(document.text_content(node))
                }
            }
            "innerHTML" => JsValue::string(
                n.children
                    .iter()
                    .map(|&id| serialize(document, id))
                    .collect::<String>(),
            ),
            "outerHTML" => JsValue::string(serialize(document, node)),
            "id" => JsValue::string(document.get_attribute(node, "id").unwrap_or("")),
            "className" => JsValue::string(document.get_attribute(node, "class").unwrap_or("")),
            "classList" => vm.host_object(node_host(node, 2), "DOMTokenList")?,
            "style" => vm.host_object(node_host(node, 1), "CSSStyleDeclaration")?,
            "value" => JsValue::string(document.control_value(node)),
            "defaultValue" => JsValue::string(document.get_attribute(node, "value").unwrap_or("")),
            "checked" => JsValue::Bool(document.control_checked(node)),
            "defaultChecked" => JsValue::Bool(document.get_attribute(node, "checked").is_some()),
            "type" => JsValue::string(document.get_attribute(node, "type").unwrap_or(
                if document.is_element_named(node, "input") {
                    "text"
                } else {
                    ""
                },
            )),
            "disabled" | "required" | "hidden" | "multiple" | "async" | "defer" => {
                JsValue::Bool(document.get_attribute(node, key).is_some())
            }
            "href" | "src" => JsValue::string(
                document
                    .get_attribute(node, key)
                    .and_then(|raw| state.url.join(raw).ok())
                    .map_or_else(String::new, |u| u.to_string()),
            ),
            "alt" | "name" | "placeholder" => {
                JsValue::string(document.get_attribute(node, key).unwrap_or(""))
            }
            "documentElement" => wrap(vm, document, document.document_element())?,
            "body" => wrap(vm, document, document.body())?,
            "head" => wrap(vm, document, document.head())?,
            "title" => JsValue::string(document.title().unwrap_or_default()),
            "readyState" => JsValue::string(&state.ready),
            "visibilityState" => JsValue::string("visible"),
            "referrer" => JsValue::string(""),
            "domain" => JsValue::string(state.url.host_str().unwrap_or("")),
            "URL" | "documentURI" => JsValue::string(state.url.as_str()),
            "activeElement" => wrap(vm, document, state.focused.or_else(|| document.body()))?,
            "attributes" => {
                let mut values = Vec::new();
                if let NodeData::Element(el) = &n.data {
                    for attribute in &el.attributes {
                        let value = vm.object()?;
                        vm.set(value.clone(), "name", JsValue::string(&attribute.name))?;
                        vm.set(value.clone(), "value", JsValue::string(&attribute.value))?;
                        values.push(value);
                    }
                }
                vm.array(values)?
            }
            "validationMessage" => JsValue::string(
                document
                    .control_state(node)
                    .map_or("", |c| c.validation_message.as_str()),
            ),
            s if s.starts_with("on") => state
                .listeners
                .iter()
                .find(|l| l.node == node && l.event == s[2..])
                .map_or(JsValue::Null, |l| l.callback.clone()),
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
    fn set(&mut self, vm: &mut Vm, id: u64, key: &str, value: JsValue) -> Result<bool, JsError> {
        if id == WINDOW {
            return Ok(false);
        }
        if id >= EVENT_BASE {
            let mut state = self.state.borrow_mut();
            if let Some(event) = state.events.get_mut(&id) {
                match key {
                    "cancelBubble" => {
                        event.stopped |= value.truthy();
                        return Ok(true);
                    }
                    "returnValue" => {
                        if !value.truthy() && event.cancelable && !event.passive {
                            event.prevented = true;
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
            }
            return Ok(false);
        }
        let node = node_id(id).ok_or_else(|| error("invalid host"))?;
        let kind = id % 8;
        if kind == 3 {
            return Err(error("computed style is read-only"));
        }
        if kind == 1 {
            let value = vm.to_string(value)?;
            let mut state = self.state.borrow_mut();
            if key == "cssText" {
                state
                    .document
                    .set_attribute(node, "style", &value)
                    .map_err(dom)?;
            } else {
                set_style(&mut state.document, node, &css_name(key), &value, "")?;
            }
            state.changed();
            return Ok(true);
        }
        if kind == 2 && key == "value" {
            let value = vm.to_string(value)?;
            let mut state = self.state.borrow_mut();
            state
                .document
                .set_attribute(node, "class", &value)
                .map_err(dom)?;
            state.changed();
            return Ok(true);
        }
        if let Some(event) = key.strip_prefix("on") {
            let mut state = self.state.borrow_mut();
            state
                .listeners
                .retain(|l| !(l.node == node && l.event == event));
            if vm.callable(&value) {
                state.listeners.push(Listener {
                    node,
                    event: event.into(),
                    callback: value,
                    capture: false,
                    once: false,
                    passive: false,
                    inline: false,
                });
            }
            return Ok(true);
        }
        let reflected = matches!(
            key,
            "id" | "className"
                | "type"
                | "name"
                | "href"
                | "src"
                | "alt"
                | "placeholder"
                | "value"
                | "defaultValue"
                | "checked"
                | "defaultChecked"
                | "hidden"
                | "disabled"
                | "required"
                | "async"
                | "defer"
        );
        if reflected {
            let boolean = matches!(
                key,
                "checked"
                    | "defaultChecked"
                    | "hidden"
                    | "disabled"
                    | "required"
                    | "async"
                    | "defer"
            );
            let text = if boolean {
                String::new()
            } else {
                vm.to_string(value.clone())?
            };
            let mut state = self.state.borrow_mut();
            match key {
                "value" => state.document.set_control_value(node, text),
                "checked" => state.document.set_control_checked(node, value.truthy()),
                _ => {
                    let name = match key {
                        "className" => "class",
                        "defaultValue" => "value",
                        "defaultChecked" => "checked",
                        other => other,
                    };
                    if boolean && !value.truthy() {
                        state.document.remove_attribute(node, name).map_err(dom)?;
                    } else {
                        state
                            .document
                            .set_attribute(node, name, &text)
                            .map_err(dom)?;
                    }
                }
            }
            state.changed();
            return Ok(true);
        }
        if matches!(key, "textContent" | "innerHTML" | "outerHTML" | "title") {
            let value = if matches!(value, JsValue::Null) {
                String::new()
            } else {
                vm.to_string(value)?
            };
            let mut state = self.state.borrow_mut();
            if key == "textContent" {
                state.document.set_text_content(node, &value).map_err(dom)?;
            } else if key == "title" {
                let title = state
                    .document
                    .get_elements_by_tag_name("title")
                    .first()
                    .copied();
                let title = if let Some(title) = title {
                    title
                } else {
                    let head = state
                        .document
                        .head()
                        .or_else(|| state.document.document_element())
                        .unwrap_or(0);
                    let title = state.document.create_element("title", Namespace::Html);
                    state
                        .document
                        .append_checked(head, title, None)
                        .map_err(dom)?;
                    title
                };
                state
                    .document
                    .set_text_content(title, &value)
                    .map_err(dom)?;
            } else {
                let tag = match &state.document.get(node).data {
                    NodeData::Element(el) => el.tag_name.as_str(),
                    _ => "div",
                };
                let fragment =
                    html::parse_fragment(value.as_bytes(), &state.url, tag, Default::default());
                if key == "innerHTML" {
                    state.document.set_text_content(node, "").map_err(dom)?;
                    for child in fragment.children_of(fragment.root()) {
                        let imported = state
                            .document
                            .import_node(&fragment, child, true)
                            .map_err(dom)?;
                        state
                            .document
                            .append_checked(node, imported, None)
                            .map_err(dom)?;
                    }
                } else if let Some(parent) = state.document.get(node).parent {
                    for child in fragment.children_of(fragment.root()) {
                        let imported = state
                            .document
                            .import_node(&fragment, child, true)
                            .map_err(dom)?;
                        state
                            .document
                            .append_checked(parent, imported, Some(node))
                            .map_err(dom)?;
                    }
                    state.document.detach(node);
                }
            }
            state.changed();
            return Ok(true);
        }
        Ok(false)
    }
    fn call(
        &mut self,
        vm: &mut Vm,
        name: &str,
        this: JsValue,
        args: &[JsValue],
    ) -> Result<HostResult, JsError> {
        let host = vm.host_id(&this).unwrap_or(WINDOW);
        if (EVENT_TARGET_BASE..EVENT_BASE).contains(&host)
            && !matches!(
                name,
                "addEventListener" | "removeEventListener" | "dispatchEvent"
            )
        {
            return Err(error("method requires a Node receiver"));
        }
        let node = if (EVENT_TARGET_BASE..EVENT_BASE).contains(&host) {
            (host / 8) as NodeId
        } else {
            node_id(host).unwrap_or(0)
        };
        let result = match name {
            "EventTarget" => {
                let mut state = self.state.borrow_mut();
                state.next += 1;
                vm.host_object(EVENT_TARGET_BASE + state.next * 8, "EventTarget")?
            }
            "Node" | "Element" | "Document" | "HTMLElement" => {
                return Err(error("illegal DOM constructor"))
            }
            "Event" | "CustomEvent" | "MouseEvent" | "KeyboardEvent" | "InputEvent"
            | "SubmitEvent" | "FocusEvent" => {
                let event = string(vm, args, 0)?;
                let options = arg(args, 1);
                let bubbles = if options.object().is_some() {
                    vm.get(options.clone(), "bubbles")?.truthy()
                } else {
                    false
                };
                let cancelable = if options.object().is_some() {
                    vm.get(options.clone(), "cancelable")?.truthy()
                } else {
                    false
                };
                let detail = if options.object().is_some() {
                    vm.get(options.clone(), "detail")?
                } else {
                    JsValue::Undefined
                };
                let key = if options.object().is_some() {
                    let key = vm.get(options, "key")?;
                    if key == JsValue::Undefined {
                        String::new()
                    } else {
                        vm.to_string(key)?
                    }
                } else {
                    String::new()
                };
                let id = self
                    .state
                    .borrow_mut()
                    .make_event(event, 0, bubbles, cancelable, detail, key);
                vm.host_object(id, name)?
            }
            "createElement" => {
                let tag = string(vm, args, 0)?;
                if tag.is_empty()
                    || tag
                        .chars()
                        .any(|c| !c.is_alphanumeric() && !matches!(c, '-' | '_' | ':'))
                {
                    return Err(error("InvalidCharacterError"));
                }
                let mut state = self.state.borrow_mut();
                if state.document.node_count() >= html::DEFAULT_MAX_NODES {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "DOM node budget"));
                }
                let id = state.document.create_element(&tag, Namespace::Html);
                wrap(vm, &state.document, Some(id))?
            }
            "createTextNode" | "createDocumentFragment" => {
                let data = if name == "createTextNode" {
                    NodeData::Text(string(vm, args, 0)?)
                } else {
                    NodeData::DocumentFragment
                };
                let mut state = self.state.borrow_mut();
                if state.document.node_count() >= html::DEFAULT_MAX_NODES {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "DOM node budget"));
                }
                let id = state.document.create_node(data);
                wrap(vm, &state.document, Some(id))?
            }
            "getElementById" => {
                let id = string(vm, args, 0)?;
                let state = self.state.borrow();
                wrap(vm, &state.document, state.document.get_element_by_id(&id))?
            }
            "querySelector" | "querySelectorAll" | "matches" | "closest" => {
                let selector = string(vm, args, 0)?;
                if name == "matches" {
                    let selectors = css::selectors::parse_selector_list(&selector)
                        .map_err(|e| JsError::new(ErrorKind::SyntaxError, e.to_string()))?;
                    let state = self.state.borrow();
                    JsValue::Bool(
                        selectors
                            .iter()
                            .any(|s| s.matches(&state.document, node, Default::default())),
                    )
                } else if name == "closest" {
                    let mut current = Some(node);
                    let mut found = None;
                    while let Some(n) = current {
                        if self.query(vm, n, &selector, true)?.contains(&n) {
                            found = Some(n);
                            break;
                        }
                        current = self.state.borrow().document.get(n).parent;
                    }
                    wrap(vm, &self.state.borrow().document, found)?
                } else {
                    let nodes = self.query(vm, node, &selector, node == 0)?;
                    if name == "querySelector" {
                        wrap(vm, &self.state.borrow().document, nodes.first().copied())?
                    } else {
                        let mut values = Vec::new();
                        for n in nodes {
                            values.push(wrap(vm, &self.state.borrow().document, Some(n))?);
                        }
                        vm.array(values)?
                    }
                }
            }
            "getElementsByTagName" | "getElementsByClassName" => {
                let name_value = string(vm, args, 0)?;
                let state = self.state.borrow();
                let nodes = state
                    .document
                    .elements_from(node)
                    .into_iter()
                    .filter(|&id| id != node || node == 0)
                    .filter(|&id| {
                        if name == "getElementsByTagName" {
                            name_value == "*"
                                || state
                                    .document
                                    .is_element_named(id, &name_value.to_ascii_lowercase())
                        } else {
                            name_value.split_ascii_whitespace().all(|c| {
                                state
                                    .document
                                    .get_attribute(id, "class")
                                    .unwrap_or("")
                                    .split_ascii_whitespace()
                                    .any(|v| v == c)
                            })
                        }
                    })
                    .collect::<Vec<_>>();
                let mut values = Vec::new();
                for id in nodes {
                    values.push(wrap(vm, &state.document, Some(id))?);
                }
                vm.array(values)?
            }
            "getAttribute" | "hasAttribute" | "setAttribute" | "removeAttribute" => {
                let key = string(vm, args, 0)?.to_ascii_lowercase();
                let value = if name == "setAttribute" {
                    Some(string(vm, args, 1)?)
                } else {
                    None
                };
                let mut state = self.state.borrow_mut();
                match name {
                    "getAttribute" => state
                        .document
                        .get_attribute(node, &key)
                        .map(JsValue::string)
                        .unwrap_or(JsValue::Null),
                    "hasAttribute" => {
                        JsValue::Bool(state.document.get_attribute(node, &key).is_some())
                    }
                    "setAttribute" => {
                        state
                            .document
                            .set_attribute(node, &key, value.as_deref().unwrap_or(""))
                            .map_err(dom)?;
                        state.changed();
                        JsValue::Undefined
                    }
                    _ => {
                        state.document.remove_attribute(node, &key).map_err(dom)?;
                        state.changed();
                        JsValue::Undefined
                    }
                }
            }
            "appendChild" | "removeChild" | "insertBefore" | "replaceChild" => {
                let child = vm
                    .host_id(&arg(args, 0))
                    .and_then(node_id)
                    .ok_or_else(|| error("argument is not Node"))?;
                let reference = vm.host_id(&arg(args, 1)).and_then(node_id);
                let mut state = self.state.borrow_mut();
                if name == "removeChild" {
                    if state.document.get(child).parent != Some(node) {
                        return Err(error("NotFoundError"));
                    }
                    state.document.detach(child);
                } else {
                    state
                        .document
                        .append_checked(node, child, reference)
                        .map_err(dom)?;
                    if name == "replaceChild" {
                        let reference = reference.ok_or_else(|| error("NotFoundError"))?;
                        state.document.detach(reference);
                    }
                }
                state.changed();
                if name == "replaceChild" {
                    arg(args, 1)
                } else {
                    arg(args, 0)
                }
            }
            "append" | "prepend" => {
                for (i, value) in args.iter().enumerate() {
                    vm.checkpoint()?;
                    if i % 128 == 0 {
                        vm.check_host_bytes(self.retained_bytes())?;
                    }
                    let child = if let Some(id) = vm.host_id(value).and_then(node_id) {
                        id
                    } else {
                        let text = vm.to_string(value.clone())?;
                        if self.state.borrow().document.node_count() >= html::DEFAULT_MAX_NODES {
                            return Err(JsError::new(ErrorKind::MemoryLimit, "DOM node budget"));
                        }
                        self.state
                            .borrow_mut()
                            .document
                            .create_node(NodeData::Text(text))
                    };
                    let mut state = self.state.borrow_mut();
                    let before = if name == "prepend" {
                        state.document.get(node).children.first().copied()
                    } else {
                        None
                    };
                    state
                        .document
                        .append_checked(node, child, before)
                        .map_err(dom)?;
                    state.changed();
                }
                JsValue::Undefined
            }
            "remove" => {
                let mut state = self.state.borrow_mut();
                state.document.detach(node);
                state.changed();
                JsValue::Undefined
            }
            "cloneNode" => {
                let mut state = self.state.borrow_mut();
                let copy = state.document.clone();
                let id = state
                    .document
                    .import_node(&copy, node, arg(args, 0).truthy())
                    .map_err(dom)?;
                wrap(vm, &state.document, Some(id))?
            }
            "contains" => JsValue::Bool(
                vm.host_id(&arg(args, 0))
                    .and_then(node_id)
                    .is_some_and(|child| self.state.borrow().document.contains(node, child)),
            ),
            "insertAdjacentHTML" => {
                let position = string(vm, args, 0)?.to_ascii_lowercase();
                let source = string(vm, args, 1)?;
                let mut state = self.state.borrow_mut();
                let context = match &state.document.get(node).data {
                    NodeData::Element(el) => el.tag_name.as_str(),
                    _ => "div",
                };
                let fragment = html::parse_fragment(
                    source.as_bytes(),
                    &state.url,
                    context,
                    Default::default(),
                );
                let (parent, before) = match position.as_str() {
                    "beforebegin" => (
                        state
                            .document
                            .get(node)
                            .parent
                            .ok_or_else(|| error("NoModificationAllowedError"))?,
                        Some(node),
                    ),
                    "afterend" => {
                        let p = state
                            .document
                            .get(node)
                            .parent
                            .ok_or_else(|| error("NoModificationAllowedError"))?;
                        let siblings = &state.document.get(p).children;
                        let at = siblings.iter().position(|&id| id == node).unwrap_or(0);
                        (p, siblings.get(at + 1).copied())
                    }
                    "afterbegin" => (node, state.document.get(node).children.first().copied()),
                    "beforeend" => (node, None),
                    _ => return Err(error("SyntaxError: invalid position")),
                };
                for child in fragment.children_of(fragment.root()) {
                    let id = state
                        .document
                        .import_node(&fragment, child, true)
                        .map_err(dom)?;
                    state
                        .document
                        .append_checked(parent, id, before)
                        .map_err(dom)?;
                }
                state.changed();
                JsValue::Undefined
            }
            "add" | "removeClass" | "toggle" | "containsClass" | "replaceClass" | "item" => {
                let token = if name == "item" {
                    String::new()
                } else {
                    string(vm, args, 0)?
                };
                if name != "item" && (token.is_empty() || token.chars().any(char::is_whitespace)) {
                    return Err(error("InvalidCharacterError"));
                }
                let mut tokens = self
                    .state
                    .borrow()
                    .document
                    .get_attribute(node, "class")
                    .unwrap_or("")
                    .split_ascii_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let mut result = JsValue::Undefined;
                match name {
                    "containsClass" => {
                        return Ok(HostResult::value(JsValue::Bool(tokens.contains(&token))))
                    }
                    "item" => {
                        let i = vm.to_number(arg(args, 0))? as usize;
                        return Ok(HostResult::value(
                            tokens.get(i).map(JsValue::string).unwrap_or(JsValue::Null),
                        ));
                    }
                    "add" => {
                        for value in args {
                            let value = vm.to_string(value.clone())?;
                            if value.is_empty() || value.chars().any(char::is_whitespace) {
                                return Err(error("InvalidCharacterError"));
                            }
                            if !tokens.contains(&value) {
                                tokens.push(value);
                            }
                        }
                    }
                    "removeClass" => {
                        let mut remove = Vec::new();
                        for value in args {
                            remove.push(vm.to_string(value.clone())?);
                        }
                        tokens.retain(|v| !remove.contains(v));
                    }
                    "toggle" => {
                        let present = tokens.contains(&token);
                        let add = if args.len() > 1 {
                            arg(args, 1).truthy()
                        } else {
                            !present
                        };
                        if add && !present {
                            tokens.push(token.clone());
                        }
                        if !add {
                            tokens.retain(|v| v != &token);
                        }
                        result = JsValue::Bool(add);
                    }
                    "replaceClass" => {
                        let new = string(vm, args, 1)?;
                        if let Some(at) = tokens.iter().position(|v| v == &token) {
                            tokens[at] = new;
                            result = JsValue::Bool(true);
                        } else {
                            result = JsValue::Bool(false);
                        }
                    }
                    _ => {}
                }
                let mut state = self.state.borrow_mut();
                state
                    .document
                    .set_attribute(node, "class", &tokens.join(" "))
                    .map_err(dom)?;
                state.changed();
                result
            }
            "setProperty" | "getPropertyValue" | "removeProperty" => {
                let key = string(vm, args, 0)?;
                if name == "getPropertyValue" && host % 8 == 3 {
                    let value = self.get(vm, host, &key)?.unwrap_or(JsValue::Undefined);
                    return Ok(HostResult::value(value));
                }
                let old = style_property(&self.state.borrow().document, node, &key);
                if name != "getPropertyValue" {
                    let value = if name == "setProperty" {
                        string(vm, args, 1)?
                    } else {
                        String::new()
                    };
                    let priority = if args.len() > 2 {
                        string(vm, args, 2)?
                    } else {
                        String::new()
                    };
                    let mut state = self.state.borrow_mut();
                    set_style(&mut state.document, node, &key, &value, &priority)?;
                    state.changed();
                }
                if name == "setProperty" {
                    JsValue::Undefined
                } else {
                    JsValue::string(old)
                }
            }
            "getComputedStyle" => {
                let node = vm
                    .host_id(&arg(args, 0))
                    .and_then(node_id)
                    .ok_or_else(|| error("getComputedStyle expects Element"))?;
                vm.host_object(node_host(node, 3), "CSSStyleDeclaration")?
            }
            "matchMedia" => {
                let query = string(vm, args, 0)?;
                let result = vm.object()?;
                vm.set(result.clone(), "media", JsValue::string(&query))?;
                vm.set(
                    result.clone(),
                    "matches",
                    JsValue::Bool(
                        css::MediaQueryList::parse(&query).matches(self.state.borrow().environment),
                    ),
                )?;
                result
            }
            "addEventListener" | "removeEventListener" => {
                let event = string(vm, args, 0)?;
                let callback = arg(args, 1);
                let options = arg(args, 2);
                let capture = if options.object().is_some() {
                    vm.get(options.clone(), "capture")?.truthy()
                } else {
                    options.truthy()
                };
                let once = if name == "addEventListener" && options.object().is_some() {
                    vm.get(options.clone(), "once")?.truthy()
                } else {
                    false
                };
                let passive = if name == "addEventListener" && options.object().is_some() {
                    vm.get(options, "passive")?.truthy()
                } else {
                    false
                };
                let mut state = self.state.borrow_mut();
                if name == "removeEventListener" {
                    state.listeners.retain(|l| {
                        !(l.node == node
                            && l.event == event
                            && l.callback == callback
                            && l.capture == capture)
                    });
                } else if vm.callable(&callback)
                    && !state.listeners.iter().any(|l| {
                        l.node == node
                            && l.event == event
                            && l.callback == callback
                            && l.capture == capture
                    })
                {
                    if state.listeners.len() >= 50_000 {
                        return Err(JsError::new(ErrorKind::MemoryLimit, "listener budget"));
                    }
                    state.listeners.push(Listener {
                        node,
                        event,
                        callback,
                        capture,
                        once,
                        passive,
                        inline: false,
                    });
                }
                JsValue::Undefined
            }
            "dispatchEvent" => {
                let event = vm
                    .host_id(&arg(args, 0))
                    .filter(|&id| id >= EVENT_BASE)
                    .ok_or_else(|| error("dispatchEvent expects Event"))?;
                return self.dispatch_plan(vm, node, event);
            }
            "preventDefault" | "stopPropagation" | "stopImmediatePropagation" | "initEvent" => {
                let mut state = self.state.borrow_mut();
                let event = state
                    .events
                    .get_mut(&host)
                    .ok_or_else(|| error("Event receiver"))?;
                match name {
                    "preventDefault" if event.cancelable && !event.passive => {
                        event.prevented = true
                    }
                    "stopPropagation" => event.stopped = true,
                    "stopImmediatePropagation" => {
                        event.stopped = true;
                        event.immediate = true;
                    }
                    _ => {}
                }
                JsValue::Undefined
            }
            "click" | "focus" | "blur" => {
                let mut state = self.state.borrow_mut();
                if name == "focus" {
                    state.focused = Some(node);
                    state.changed();
                }
                if name == "blur" {
                    state.focused = None;
                    state.changed();
                }
                state.queued.push_back((node, name.into()));
                JsValue::Undefined
            }
            "submit" => {
                self.state
                    .borrow_mut()
                    .queued
                    .push_back((node, "submit".into()));
                JsValue::Undefined
            }
            "reset" => {
                let mut state = self.state.borrow_mut();
                for id in state.document.elements_from(node) {
                    if state.document.is_element_named(id, "input") {
                        let value = state
                            .document
                            .get_attribute(id, "value")
                            .unwrap_or("")
                            .to_owned();
                        let checked = state.document.get_attribute(id, "checked").is_some();
                        state.document.set_control_value(id, value);
                        state.document.set_control_checked(id, checked);
                    }
                }
                state.changed();
                JsValue::Undefined
            }
            "checkValidity" | "reportValidity" => {
                let state = self.state.borrow();
                JsValue::Bool(
                    !state
                        .document
                        .control_state(node)
                        .is_some_and(|s| !s.validation_message.is_empty())
                        && (!state.document.get_attribute(node, "required").is_some()
                            || !state.document.control_value(node).is_empty()),
                )
            }
            "setCustomValidity" => {
                let message = string(vm, args, 0)?;
                self.state
                    .borrow_mut()
                    .document
                    .control_state_mut(node)
                    .validation_message = message;
                JsValue::Undefined
            }
            "setTimeout" | "setInterval" | "requestAnimationFrame" => {
                let callback = arg(args, 0);
                if !vm.callable(&callback) && !matches!(callback, JsValue::String(_)) {
                    return Err(error("timer callback"));
                }
                if matches!(callback, JsValue::String(_)) && !self.allow_dynamic_code() {
                    return Err(error("CSP blocks string timer"));
                }
                let delay = if name == "requestAnimationFrame" {
                    16.0
                } else {
                    let n = vm.to_number(arg(args, 1))?;
                    if n.is_nan() {
                        0.0
                    } else {
                        n.clamp(0.0, 86400000.0)
                    }
                };
                let delay = Duration::from_secs_f64(delay / 1000.0);
                let mut state = self.state.borrow_mut();
                if state.timers.len() >= 10000 {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "timer budget"));
                }
                state.next += 1;
                let id = state.next;
                let args = if name == "requestAnimationFrame" {
                    vec![JsValue::Number(0.0)]
                } else {
                    args.iter().skip(2).cloned().collect()
                };
                state.timers.insert(
                    id,
                    Timer {
                        callback,
                        args,
                        due: Instant::now() + delay,
                        interval: if name == "setInterval" {
                            Some(delay.max(Duration::from_millis(4)))
                        } else {
                            None
                        },
                        animation_frame: name == "requestAnimationFrame",
                    },
                );
                JsValue::Number(id as f64)
            }
            "clearTimeout" | "clearInterval" | "cancelAnimationFrame" => {
                let id = vm.to_number(arg(args, 0))? as u64;
                self.state.borrow_mut().timers.remove(&id);
                if self.state.borrow().running_timer == Some(id) {
                    self.state.borrow_mut().cancel_running_timer = true;
                }
                JsValue::Undefined
            }
            _ => return Err(error(format!("unknown host method {name}"))),
        };
        Ok(HostResult::value(result))
    }
    fn before_callback(
        &mut self,
        vm: &mut Vm,
        _function: &JsValue,
        _this: &JsValue,
        args: &[JsValue],
    ) -> bool {
        let Some(id) = vm.host_id(&arg(args, 0)).filter(|&id| id >= EVENT_BASE) else {
            return true;
        };
        let mut state = self.state.borrow_mut();
        let Some(event) = state.events.get_mut(&id) else {
            return false;
        };
        let Some(listener) = event.queue.pop_front() else {
            return false;
        };
        if event.immediate || event.stopped && event.current != listener.node {
            return false;
        }
        if !listener.inline
            && !state.listeners.iter().any(|l| {
                l.node == listener.node
                    && l.event == listener.event
                    && l.callback == listener.callback
                    && l.capture == listener.capture
            })
        {
            return false;
        }
        let Some(event) = state.events.get_mut(&id) else {
            return false;
        };
        event.current = listener.node;
        event.phase = if listener.node == event.target {
            2
        } else if listener.capture {
            1
        } else {
            3
        };
        event.passive = listener.passive;
        if listener.once {
            state.listeners.retain(|l| {
                !(l.node == listener.node
                    && l.event == listener.event
                    && l.callback == listener.callback
                    && l.capture == listener.capture)
            });
        }
        true
    }
    fn finish(&mut self, _vm: &mut Vm, token: u64) -> Result<JsValue, JsError> {
        let mut state = self.state.borrow_mut();
        let Some(event) = state.events.get_mut(&token) else {
            return Ok(JsValue::Bool(true));
        };
        event.phase = 0;
        event.passive = false;
        let prevented = event.prevented;
        let node = event.target;
        let name = event.name.clone();
        let old_checked = event.old_checked;
        if name == "click" {
            if let Some(old) = old_checked {
                if prevented {
                    state.document.set_control_checked(node, old);
                } else {
                    state.queued.push_back((node, "input".into()));
                    state.queued.push_back((node, "change".into()));
                }
                state.changed();
            }
            if !prevented
                && state.document.is_element_named(node, "button")
                && state.document.get_attribute(node, "type") != Some("button")
            {
                let mut parent = state.document.get(node).parent;
                while let Some(id) = parent {
                    if state.document.is_element_named(id, "form") {
                        state.queued.push_back((id, "submit".into()));
                        break;
                    }
                    parent = state.document.get(id).parent;
                }
            }
        }
        Ok(JsValue::Bool(!prevented))
    }
    fn roots(&self, live: &HashSet<u64>) -> Vec<JsValue> {
        let state = self.state.borrow();
        let mut roots = state.functions.values().cloned().collect::<Vec<_>>();
        for listener in &state.listeners {
            if state.document.contains(0, listener.node)
                || live.contains(&node_host(listener.node, 0))
            {
                roots.push(listener.callback.clone());
            }
        }
        for timer in state.timers.values() {
            roots.push(timer.callback.clone());
            roots.extend(timer.args.clone());
        }
        for (&id, event) in &state.events {
            if live.contains(&id) {
                roots.push(event.detail.clone());
            }
        }
        roots
    }
    fn host_references(&self, live: &HashSet<u64>) -> Vec<u64> {
        self.state
            .borrow()
            .events
            .iter()
            .filter(|(id, _)| live.contains(id))
            .map(|(_, event)| node_host(event.target, 0))
            .collect()
    }
    fn sweep(&mut self, live: &HashSet<u64>) {
        let mut state = self.state.borrow_mut();
        if state.parsing {
            return;
        }
        let roots = live
            .iter()
            .filter_map(|&id| node_id(id))
            .collect::<HashSet<_>>();
        let connected = state
            .document
            .elements_from(0)
            .into_iter()
            .collect::<HashSet<_>>();
        state.listeners.retain(|l| {
            connected.contains(&l.node)
                || live.contains(&node_host(l.node, 0))
                || roots.contains(&l.node)
        });
        state.events.retain(|id, _| live.contains(id));
        state.document.sweep_detached(&roots);
        if state
            .focused
            .is_some_and(|node| !state.document.contains(0, node))
        {
            state.focused = None;
            state.changed();
        }
    }
}
fn css_name(s: &str) -> String {
    if s == "cssFloat" {
        return "float".into();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
fn style_property(doc: &Document, node: NodeId, key: &str) -> String {
    css::syntax::parse_declarations(doc.get_attribute(node, "style").unwrap_or(""))
        .into_iter()
        .rev()
        .find(|d| d.name == key)
        .map(|d| css::token::serialize(&d.value))
        .unwrap_or_default()
}
fn set_style(
    doc: &mut Document,
    node: NodeId,
    key: &str,
    value: &str,
    priority: &str,
) -> Result<(), JsError> {
    if !priority.is_empty() && !priority.eq_ignore_ascii_case("important") {
        return Ok(());
    }
    let mut declarations =
        css::syntax::parse_declarations(doc.get_attribute(node, "style").unwrap_or(""))
            .into_iter()
            .filter(|d| d.name != key)
            .map(|d| {
                format!(
                    "{}:{}{}",
                    d.name,
                    css::token::serialize(&d.value),
                    if d.important { "!important" } else { "" }
                )
            })
            .collect::<Vec<_>>();
    if !value.is_empty() {
        declarations.push(format!(
            "{key}:{value}{}",
            if priority.eq_ignore_ascii_case("important") {
                "!important"
            } else {
                ""
            }
        ));
    }
    doc.set_attribute(node, "style", &declarations.join(";"))
        .map_err(dom)
}
fn serialize(doc: &Document, node: NodeId) -> String {
    fn escape(s: &str, attribute: bool) -> String {
        let s = s
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        if attribute {
            s.replace('"', "&quot;")
        } else {
            s
        }
    }
    match &doc.get(node).data {
        NodeData::Text(s) => escape(s, false),
        NodeData::Comment(s) => format!("<!--{s}-->"),
        NodeData::Element(el) => {
            let attributes = el
                .attributes
                .iter()
                .map(|a| format!(" {}=\"{}\"", a.name, escape(&a.value, true)))
                .collect::<String>();
            let open = format!("<{}{attributes}>", el.tag_name);
            if matches!(
                el.tag_name.as_str(),
                "input"
                    | "img"
                    | "br"
                    | "hr"
                    | "meta"
                    | "link"
                    | "area"
                    | "base"
                    | "source"
                    | "wbr"
            ) {
                open
            } else {
                format!(
                    "{open}{}</{}>",
                    doc.children_of(node)
                        .into_iter()
                        .map(|c| serialize(doc, c))
                        .collect::<String>(),
                    el.tag_name
                )
            }
        }
        _ => doc
            .children_of(node)
            .into_iter()
            .map(|c| serialize(doc, c))
            .collect(),
    }
}
