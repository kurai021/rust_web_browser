//! Standard data/DOM adapters used by modern apps; no URL/site-specific paths.
use crate::*;
use js::{value::Property, ErrorKind};
pub(crate) fn install(vm: &mut Vm, state: &Rc<RefCell<State>>) -> Result<(), JsError> {
    let array = vm.get_global("Array")?;
    let array = vm.get(array, "prototype")?;
    for name in ["NodeList", "DOMParser"] {
        let constructor = vm.native(&format!("host:{name}"))?;
        let prototype = vm.object()?;
        vm.set(prototype.clone(), "constructor", constructor.clone())?;
        let symbol = vm.get_global("Symbol")?;
        let tag = vm.get(symbol, "toStringTag")?;
        let tag = vm.property_key(tag)?;
        vm.define(
            prototype
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::Error, "DOM prototype"))?,
            tag,
            Property {
                enumerable: false,
                ..Property::data(JsValue::string(name))
            },
        )?;
        if name == "NodeList" {
            for method in ["forEach", "keys", "values", "entries"] {
                let function = vm.get(array.clone(), method)?;
                vm.set(prototype.clone(), method, function)?;
            }
            let symbol = vm.get_global("Symbol")?;
            let iterator = vm.get(symbol, "iterator")?;
            let iterator = vm.property_key(iterator)?;
            let values = vm.get(prototype.clone(), "values")?;
            vm.define(
                prototype.object().unwrap(),
                iterator,
                Property {
                    enumerable: false,
                    ..Property::data(values)
                },
            )?;
            let item = vm.native("host:NodeList.item")?;
            vm.set(prototype.clone(), "item", item)?;
        } else {
            let function = vm.native("host:parseFromString")?;
            state
                .borrow_mut()
                .functions
                .insert("parseFromString".into(), function.clone());
            vm.set(prototype.clone(), "parseFromString", function)?;
        }
        vm.define(
            constructor
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::Error, "DOM constructor"))?,
            "prototype".into(),
            Property {
                writable: false,
                enumerable: false,
                configurable: false,
                ..Property::data(prototype.clone())
            },
        )?;
        state
            .borrow_mut()
            .functions
            .insert(format!("{name}.prototype"), prototype);
        vm.set_global(name, constructor)?;
    }
    Ok(())
}
pub(crate) fn list(vm: &mut Vm, state: &State, values: Vec<JsValue>) -> Result<JsValue, JsError> {
    let value = vm.object()?;
    let id = value
        .object()
        .ok_or_else(|| JsError::new(ErrorKind::Error, "NodeList allocation"))?;
    let length = values.len();
    for (index, item) in values.into_iter().enumerate() {
        vm.define(
            id,
            index.to_string().into(),
            Property {
                writable: false,
                enumerable: true,
                configurable: true,
                ..Property::data(item)
            },
        )?;
    }
    vm.define(
        id,
        "length".into(),
        Property {
            writable: false,
            enumerable: false,
            configurable: true,
            ..Property::data(JsValue::Number(length as f64))
        },
    )?;
    if let Some(prototype) = state.functions.get("NodeList.prototype") {
        vm.set_host_prototype(&value, prototype.clone())?;
    }
    Ok(value)
}
pub(crate) fn data_attribute(key: &str) -> Result<String, JsError> {
    if key
        .as_bytes()
        .windows(2)
        .any(|w| w[0] == b'-' && w[1].is_ascii_lowercase())
    {
        return Err(JsError::new(
            ErrorKind::SyntaxError,
            "invalid DOMStringMap name",
        ));
    }
    let mut name = "data-".to_owned();
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            name.push('-');
            name.push(ch.to_ascii_lowercase());
        } else {
            name.push(ch);
        }
    }
    Ok(name)
}
pub(crate) fn parse(
    vm: &mut Vm,
    state: &Rc<RefCell<State>>,
    source: String,
) -> Result<JsValue, JsError> {
    let url = state.borrow().url.clone();
    let parsed = html::parse_full(
        source.as_bytes(),
        &url,
        html::ParseOpts {
            scripting_enabled: false,
            ..Default::default()
        },
    );
    if parsed.truncated.is_some() {
        return Err(JsError::new(
            ErrorKind::MemoryLimit,
            "inert document parser limit",
        ));
    }
    vm.check_host_bytes(
        state
            .borrow()
            .document
            .retained_bytes()
            .saturating_add(parsed.retained_bytes()),
    )?;
    let mut state = state.borrow_mut();
    if state
        .document
        .node_count()
        .saturating_add(parsed.node_count())
        > html::DEFAULT_MAX_NODES
    {
        return Err(JsError::new(ErrorKind::MemoryLimit, "DOM node budget"));
    }
    let root = state
        .document
        .import_node(&parsed, 0, true)
        .map_err(|e| JsError::new(ErrorKind::Error, e.to_string()))?;
    state.inert_documents.insert(root);
    state.adopt(root, root);
    vm.host_object(node_host(root, 0), "HTMLDocument")
}
pub(crate) fn replace_children(
    vm: &mut Vm,
    state: &Rc<RefCell<State>>,
    parent: NodeId,
    args: &[JsValue],
) -> Result<(), JsError> {
    let mut children = Vec::new();
    for value in args {
        vm.checkpoint()?;
        let child = if let Some(node) = vm.host_id(value).and_then(node_id) {
            node
        } else {
            let text = vm.to_string(value.clone())?;
            let mut state = state.borrow_mut();
            if state.document.node_count() >= html::DEFAULT_MAX_NODES {
                return Err(JsError::new(ErrorKind::MemoryLimit, "DOM node budget"));
            }
            let node = state.document.create_node(html::NodeData::Text(text));
            state.adopt(node, parent);
            node
        };
        children.push(child);
    }
    let mut state = state.borrow_mut();
    if !state.document.try_get(parent).is_some_and(|n| {
        matches!(
            n.data,
            html::NodeData::Document
                | html::NodeData::DocumentFragment
                | html::NodeData::Element(_)
        )
    }) {
        return Err(JsError::new(ErrorKind::TypeError, "HierarchyRequestError"));
    }
    for child in &children {
        if state.document.contains(*child, parent)
            || state
                .document
                .try_get(*child)
                .is_none_or(|n| matches!(n.data, html::NodeData::Document))
        {
            return Err(JsError::new(ErrorKind::TypeError, "HierarchyRequestError"));
        }
    }
    for child in state.document.children_of(parent) {
        state.document.detach(child);
    }
    for child in children {
        state.adopt(child, parent);
        state
            .document
            .append_checked(parent, child, None)
            .map_err(|e| JsError::new(ErrorKind::Error, e.to_string()))?;
    }
    state.changed();
    Ok(())
}
