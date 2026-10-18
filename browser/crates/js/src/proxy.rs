//! Basic Proxy internal methods and Reflect. Traps run outside arena borrows;
//! non-configurable descriptors and non-extensible target invariants are checked.
use super::descriptors::Descriptor;
use super::*;
#[derive(Debug, Clone)]
pub(crate) struct ProxyData {
    pub target: ObjectId,
    pub handler: Option<JsValue>,
    pub callable: bool,
    pub constructible: bool,
}
fn error(message: &str) -> JsError {
    JsError::new(ErrorKind::TypeError, message)
}
pub(crate) fn same(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Number(a), JsValue::Number(b)) if a.is_nan() && b.is_nan() => true,
        (JsValue::Number(a), JsValue::Number(b)) if *a == 0.0 && *b == 0.0 => {
            a.is_sign_negative() == b.is_sign_negative()
        }
        _ => strict_equal(a, b),
    }
}
fn key_value(key: &PropertyKey) -> JsValue {
    match key {
        PropertyKey::String(s) => JsValue::string(s),
        PropertyKey::Symbol(id) => JsValue::Symbol(*id, "".into()),
    }
}
fn compatible(current: Option<&Property>, extensible: bool, new: &Property) -> bool {
    let Some(old) = current else {
        return extensible;
    };
    if old.configurable {
        return true;
    }
    if new.configurable || new.enumerable != old.enumerable {
        return false;
    }
    match (&old.value, &new.value) {
        (PropertyValue::Data(a), PropertyValue::Data(b)) => {
            old.writable || !new.writable && same(a, b)
        }
        (
            PropertyValue::Accessor { get: a, set: b },
            PropertyValue::Accessor { get: c, set: d },
        ) => same(a, c) && same(b, d),
        _ => false,
    }
}
impl Vm {
    pub(super) fn install_proxy(&mut self) -> Result<(), JsError> {
        let constructor = self.native("Proxy")?;
        if let Some(id) = constructor.object() {
            let revoke = self.native("Proxy.revocable")?;
            self.define(
                id,
                "revocable".into(),
                Property {
                    enumerable: false,
                    ..Property::data(revoke)
                },
            )?;
        }
        self.set_global("Proxy", constructor)?;
        let reflect = self.object()?;
        for name in [
            "get",
            "set",
            "has",
            "deleteProperty",
            "ownKeys",
            "defineProperty",
            "getOwnPropertyDescriptor",
            "getPrototypeOf",
            "setPrototypeOf",
            "isExtensible",
            "preventExtensions",
            "apply",
            "construct",
        ] {
            let value = self.native(&format!("Reflect.{name}"))?;
            self.define(
                reflect.object().ok_or_else(|| error("Reflect intrinsic"))?,
                name.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        self.set_global("Reflect", reflect)
    }
    fn proxy_operation<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, JsError>,
    ) -> Result<T, JsError> {
        if self.proxy_depth >= 128 {
            return Err(JsError::new(ErrorKind::RangeError, "proxy chain depth"));
        }
        self.proxy_depth += 1;
        let result = f(self);
        self.proxy_depth -= 1;
        result
    }
    fn proxy_trap(
        &mut self,
        id: ObjectId,
        name: &str,
    ) -> Result<(JsValue, JsValue, JsValue), JsError> {
        let ObjectKind::Proxy(data) = self.heap.get(id)?.kind.clone() else {
            return Err(error("Proxy receiver"));
        };
        let handler = data.handler.ok_or_else(|| error("revoked Proxy"))?;
        let method = self.get(handler.clone(), name)?;
        if !matches!(method, JsValue::Undefined | JsValue::Null) && !self.callable(&method) {
            return Err(error("Proxy trap is not callable"));
        }
        Ok((JsValue::Object(data.target), handler, method))
    }
    pub(crate) fn proxy_get(
        &mut self,
        id: ObjectId,
        key: PropertyKey,
        receiver: JsValue,
    ) -> Result<JsValue, JsError> {
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "get")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.get_key_receiver(target, key, receiver);
            }
            let value = vm.call_inner(
                trap,
                handler,
                vec![target.clone(), key_value(&key), receiver],
                false,
                None,
            )?;
            if let Some(desc) = vm.own_descriptor(target, &key)? {
                if !desc.configurable {
                    match desc.value {
                        PropertyValue::Data(current)
                            if !desc.writable && !same(&current, &value) =>
                        {
                            return Err(error("Proxy get invariant"))
                        }
                        PropertyValue::Accessor { get, .. }
                            if get == JsValue::Undefined && value != JsValue::Undefined =>
                        {
                            return Err(error("Proxy getter invariant"))
                        }
                        _ => {}
                    }
                }
            }
            Ok(value)
        })
    }
    pub(crate) fn reflect_set(
        &mut self,
        target: JsValue,
        key: PropertyKey,
        value: JsValue,
        receiver: JsValue,
    ) -> Result<bool, JsError> {
        let id = target.object().ok_or_else(|| error("Reflect.set target"))?;
        if matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            return self.proxy_set(id, key, value, receiver);
        }
        if matches!(self.heap.get(id)?.kind, ObjectKind::ModuleNamespace(_)) {
            return Ok(false);
        }
        let descriptor = self.own_descriptor(target.clone(), &key)?;
        let descriptor = if let Some(desc) = descriptor {
            Some(desc)
        } else if let Some(parent) = self.object_prototype(target.clone())?.object() {
            return self.reflect_set(JsValue::Object(parent), key, value, receiver);
        } else {
            None
        };
        if let Some(Property {
            value: PropertyValue::Accessor { set, .. },
            ..
        }) = descriptor
        {
            if !self.callable(&set) {
                return Ok(false);
            }
            self.call_inner(set, receiver, vec![value], false, None)?;
            return Ok(true);
        }
        if descriptor.as_ref().is_some_and(|d| !d.writable) || receiver.object().is_none() {
            return Ok(false);
        }
        if target == receiver
            && matches!(
                self.heap.get(id)?.kind,
                ObjectKind::Host { .. } | ObjectKind::Array
            )
        {
            self.set_key(receiver, key, value)?;
            return Ok(true);
        }
        let own = self.own_descriptor(receiver.clone(), &key)?;
        let descriptor = if let Some(own) = own {
            if !own.writable || matches!(own.value, PropertyValue::Accessor { .. }) {
                return Ok(false);
            }
            Descriptor {
                value: Some(value),
                ..Descriptor::default()
            }
        } else {
            Descriptor::full(Property::data(value))
        };
        self.reflect_define_partial(receiver, key, descriptor)
    }
    pub(crate) fn proxy_set(
        &mut self,
        id: ObjectId,
        key: PropertyKey,
        value: JsValue,
        receiver: JsValue,
    ) -> Result<bool, JsError> {
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "set")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.reflect_set(target, key, value, receiver);
            }
            let success = vm
                .call_inner(
                    trap,
                    handler,
                    vec![target.clone(), key_value(&key), value.clone(), receiver],
                    false,
                    None,
                )?
                .truthy();
            if !success {
                return Ok(false);
            }
            if let Some(desc) = vm.own_descriptor(target, &key)? {
                if !desc.configurable {
                    match desc.value {
                        PropertyValue::Data(current)
                            if !desc.writable && !same(&current, &value) =>
                        {
                            return Err(error("Proxy set invariant"))
                        }
                        PropertyValue::Accessor {
                            set: JsValue::Undefined,
                            ..
                        } => return Err(error("Proxy setter invariant")),
                        _ => {}
                    }
                }
            }
            Ok(true)
        })
    }
    pub(crate) fn proxy_has(&mut self, id: ObjectId, key: PropertyKey) -> Result<bool, JsError> {
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "has")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.has(target, key);
            }
            let present = vm
                .call_inner(
                    trap,
                    handler,
                    vec![target.clone(), key_value(&key)],
                    false,
                    None,
                )?
                .truthy();
            if !present {
                if let Some(desc) = vm.own_descriptor(target.clone(), &key)? {
                    if !desc.configurable || !vm.object_extensible(target)? {
                        return Err(error("Proxy has invariant"));
                    }
                }
            }
            Ok(present)
        })
    }
    pub(crate) fn proxy_delete(&mut self, id: ObjectId, key: PropertyKey) -> Result<bool, JsError> {
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "deleteProperty")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.delete(target, key);
            }
            let success = vm
                .call_inner(
                    trap,
                    handler,
                    vec![target.clone(), key_value(&key)],
                    false,
                    None,
                )?
                .truthy();
            if success {
                if let Some(desc) = vm.own_descriptor(target.clone(), &key)? {
                    if !desc.configurable || !vm.object_extensible(target)? {
                        return Err(error("Proxy delete invariant"));
                    }
                }
            }
            Ok(success)
        })
    }
    pub(crate) fn own_descriptor(
        &mut self,
        target: JsValue,
        key: &PropertyKey,
    ) -> Result<Option<Property>, JsError> {
        let Some(id) = target.object() else {
            if matches!(target, JsValue::Undefined | JsValue::Null) {
                return Err(error("descriptor target"));
            }
            if let (JsValue::String(text), PropertyKey::String(key)) = (&target, key) {
                if key == "length" {
                    return Ok(Some(Property {
                        writable: false,
                        enumerable: false,
                        configurable: false,
                        ..Property::data(JsValue::Number(text.encode_utf16().count() as f64))
                    }));
                }
                if let Ok(index) = key.parse::<usize>() {
                    if index.to_string() == *key {
                        if let Some(unit) = text.encode_utf16().nth(index) {
                            return Ok(Some(Property {
                                writable: false,
                                enumerable: true,
                                configurable: false,
                                ..Property::data(JsValue::string(String::from_utf16_lossy(&[unit])))
                            }));
                        }
                    }
                }
            }
            return Ok(None);
        };
        if let ObjectKind::ModuleNamespace(module) = self.heap.get(id)?.kind {
            if let PropertyKey::String(name) = key {
                if self.module_names(module)?.contains(name) {
                    return Ok(Some(Property {
                        value: PropertyValue::Data(self.module_get(module, key)?),
                        writable: true,
                        enumerable: true,
                        configurable: false,
                    }));
                }
            }
        }
        if !matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            return Ok(self.heap.get(id)?.properties.get(key).cloned());
        }
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "getOwnPropertyDescriptor")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.own_descriptor(target, key);
            }
            let result = vm.call_inner(
                trap,
                handler,
                vec![target.clone(), key_value(key)],
                false,
                None,
            )?;
            let current = vm.own_descriptor(target.clone(), key)?;
            let extensible = vm.object_extensible(target)?;
            if result == JsValue::Undefined {
                if current.as_ref().is_some_and(|d| !d.configurable)
                    || !extensible && current.is_some()
                {
                    return Err(error("Proxy descriptor hiding invariant"));
                }
                return Ok(None);
            }
            let descriptor = crate::builtins::descriptor(vm, result)?;
            if !compatible(current.as_ref(), extensible, &descriptor)
                || !descriptor.configurable && current.as_ref().is_none_or(|d| d.configurable)
            {
                return Err(error("Proxy descriptor invariant"));
            }
            Ok(Some(descriptor))
        })
    }
    pub(crate) fn descriptor_object(&mut self, property: Property) -> Result<JsValue, JsError> {
        let object = self.object()?;
        let id = object
            .object()
            .ok_or_else(|| error("descriptor allocation"))?;
        let mut props = vec![
            ("enumerable", JsValue::Bool(property.enumerable)),
            ("configurable", JsValue::Bool(property.configurable)),
        ];
        match property.value {
            PropertyValue::Data(value) => props.extend([
                ("value", value),
                ("writable", JsValue::Bool(property.writable)),
            ]),
            PropertyValue::Accessor { get, set } => props.extend([("get", get), ("set", set)]),
        }
        for (name, value) in props {
            self.define(id, name.into(), Property::data(value))?;
        }
        Ok(object)
    }
    pub(crate) fn reflect_define(
        &mut self,
        target: JsValue,
        key: PropertyKey,
        descriptor: Property,
    ) -> Result<bool, JsError> {
        self.reflect_define_partial(target, key, Descriptor::full(descriptor))
    }
    pub(crate) fn reflect_define_partial(
        &mut self,
        target: JsValue,
        key: PropertyKey,
        descriptor: Descriptor,
    ) -> Result<bool, JsError> {
        let id = target.object().ok_or_else(|| error("define target"))?;
        if !matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            let current = self.own_descriptor(target.clone(), &key)?;
            if matches!(self.heap.get(id)?.kind, ObjectKind::ModuleNamespace(_))
                && matches!(key, PropertyKey::String(_))
            {
                let Some(current) = current else {
                    return Ok(false);
                };
                let same_value = match (&descriptor.value, &current.value) {
                    (Some(new), PropertyValue::Data(old)) => same(new, old),
                    _ => true,
                };
                return Ok(!descriptor.accessor()
                    && descriptor.writable != Some(false)
                    && descriptor.enumerable != Some(false)
                    && descriptor.configurable != Some(true)
                    && same_value);
            }
            let complete = descriptor.complete(current.as_ref());
            if !compatible(current.as_ref(), self.heap.get(id)?.extensible, &complete) {
                return Ok(false);
            }
            self.define(id, key, complete)?;
            return Ok(true);
        }
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "defineProperty")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.reflect_define_partial(target, key, descriptor);
            }
            let desc = vm.partial_descriptor_object(&descriptor)?;
            let success = vm
                .call_inner(
                    trap,
                    handler,
                    vec![target.clone(), key_value(&key), desc],
                    false,
                    None,
                )?
                .truthy();
            if success {
                let current = vm.own_descriptor(target.clone(), &key)?;
                if !compatible(
                    current.as_ref(),
                    vm.object_extensible(target)?,
                    &descriptor.complete(current.as_ref()),
                ) || descriptor.configurable == Some(false)
                    && current.as_ref().is_none_or(|d| d.configurable)
                {
                    return Err(error("Proxy define invariant"));
                }
            }
            Ok(success)
        })
    }
    pub(crate) fn proxy_keys(
        &mut self,
        id: ObjectId,
        enumerable: bool,
    ) -> Result<Vec<PropertyKey>, JsError> {
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "ownKeys")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.keys(&target, enumerable);
            }
            let result = vm.call_inner(trap, handler, vec![target.clone()], false, None)?;
            if result.object().is_none() {
                return Err(error("Proxy ownKeys result"));
            }
            let values = vm.array_values(result)?;
            let mut keys = Vec::new();
            let mut set = HashSet::new();
            for value in values {
                let key = match value {
                    JsValue::String(v) => PropertyKey::String(v.to_string()),
                    JsValue::Symbol(id, _) => PropertyKey::Symbol(id),
                    _ => return Err(error("Proxy ownKeys key type")),
                };
                if !set.insert(key.clone()) {
                    return Err(error("Proxy ownKeys duplicates"));
                }
                keys.push(key);
            }
            let target_keys = vm.keys(&target, false)?;
            for key in &target_keys {
                if !set.contains(key)
                    && vm
                        .own_descriptor(target.clone(), key)?
                        .is_some_and(|p| !p.configurable)
                {
                    return Err(error("Proxy ownKeys invariant"));
                }
            }
            if !vm.object_extensible(target)?
                && (keys.len() != target_keys.len() || target_keys.iter().any(|k| !set.contains(k)))
            {
                return Err(error("Proxy ownKeys nonextensible invariant"));
            }
            if enumerable {
                let proxy = JsValue::Object(id);
                let mut out = Vec::new();
                for key in keys {
                    if vm
                        .own_descriptor(proxy.clone(), &key)?
                        .is_some_and(|d| d.enumerable)
                    {
                        out.push(key);
                    }
                }
                Ok(out)
            } else {
                Ok(keys)
            }
        })
    }
    pub(crate) fn object_prototype(&mut self, target: JsValue) -> Result<JsValue, JsError> {
        let id = target.object().ok_or_else(|| error("prototype target"))?;
        if !matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            return Ok(self
                .heap
                .get(id)?
                .prototype
                .map(JsValue::Object)
                .unwrap_or(JsValue::Null));
        }
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "getPrototypeOf")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.object_prototype(target);
            }
            let result = vm.call_inner(trap, handler, vec![target.clone()], false, None)?;
            if result != JsValue::Null && result.object().is_none() {
                return Err(error("Proxy prototype type"));
            }
            if !vm.object_extensible(target.clone())? && vm.object_prototype(target)? != result {
                return Err(error("Proxy prototype invariant"));
            }
            Ok(result)
        })
    }
    pub(crate) fn set_object_prototype(
        &mut self,
        target: JsValue,
        prototype: JsValue,
    ) -> Result<bool, JsError> {
        let id = target.object().ok_or_else(|| error("prototype target"))?;
        if prototype != JsValue::Null && prototype.object().is_none() {
            return Err(error("prototype type"));
        }
        if matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            return self.proxy_operation(|vm| {
                let (target, handler, trap) = vm.proxy_trap(id, "setPrototypeOf")?;
                if matches!(trap, JsValue::Undefined | JsValue::Null) {
                    return vm.set_object_prototype(target, prototype);
                }
                let success = vm
                    .call_inner(
                        trap,
                        handler,
                        vec![target.clone(), prototype.clone()],
                        false,
                        None,
                    )?
                    .truthy();
                if success
                    && !vm.object_extensible(target.clone())?
                    && vm.object_prototype(target)? != prototype
                {
                    return Err(error("Proxy setPrototype invariant"));
                }
                Ok(success)
            });
        }
        if self.heap.get(id)?.prototype == prototype.object() {
            return Ok(true);
        }
        if !self.heap.get(id)?.extensible {
            return Ok(false);
        }
        let mut current = prototype.object();
        let mut seen = HashSet::new();
        while let Some(parent) = current {
            if parent == id || !seen.insert(parent) {
                return Ok(false);
            }
            current = self.object_prototype(JsValue::Object(parent))?.object();
        }
        self.heap.get_mut(id)?.prototype = prototype.object();
        Ok(true)
    }
    pub(crate) fn object_extensible(&mut self, target: JsValue) -> Result<bool, JsError> {
        let id = target.object().ok_or_else(|| error("extensible target"))?;
        if !matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            return Ok(self.heap.get(id)?.extensible);
        }
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "isExtensible")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.object_extensible(target);
            }
            let result = vm
                .call_inner(trap, handler, vec![target.clone()], false, None)?
                .truthy();
            if vm.object_extensible(target)? != result {
                return Err(error("Proxy extensible invariant"));
            }
            Ok(result)
        })
    }
    pub(crate) fn prevent_extensions(&mut self, target: JsValue) -> Result<bool, JsError> {
        let id = target
            .object()
            .ok_or_else(|| error("preventExtensions target"))?;
        if !matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            self.heap.get_mut(id)?.extensible = false;
            return Ok(true);
        }
        self.proxy_operation(|vm| {
            let (target, handler, trap) = vm.proxy_trap(id, "preventExtensions")?;
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return vm.prevent_extensions(target);
            }
            let result = vm
                .call_inner(trap, handler, vec![target.clone()], false, None)?
                .truthy();
            if result && vm.object_extensible(target)? {
                return Err(error("Proxy preventExtensions invariant"));
            }
            Ok(result)
        })
    }
    pub(crate) fn proxy_call(
        &mut self,
        id: ObjectId,
        this: JsValue,
        args: Vec<JsValue>,
        construct: bool,
    ) -> Result<JsValue, JsError> {
        self.proxy_operation(|vm| {
            let (target, handler, trap) =
                vm.proxy_trap(id, if construct { "construct" } else { "apply" })?;
            if !vm.callable(&target) {
                return Err(error("Proxy target is not callable"));
            }
            if matches!(trap, JsValue::Undefined | JsValue::Null) {
                return if construct {
                    vm.construct(target, args)
                } else {
                    vm.call_inner(target, this, args, false, None)
                };
            }
            let args = vm.array(args)?;
            let result = vm.call_inner(
                trap,
                handler,
                if construct {
                    vec![target, args, JsValue::Object(id)]
                } else {
                    vec![target, this, args]
                },
                false,
                None,
            )?;
            if construct && result.object().is_none() {
                return Err(error("Proxy construct result"));
            }
            Ok(result)
        })
    }
    pub(crate) fn proxy_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
        construct: bool,
    ) -> Option<Result<JsValue, JsError>> {
        let _ = this;
        let arg = |i| args.get(i).cloned().unwrap_or(JsValue::Undefined);
        if matches!(name, "Proxy" | "Proxy.revocable") {
            return Some((|| {
                if name == "Proxy" && !construct {
                    return Err(error("Proxy requires new"));
                }
                let target = arg(0)
                    .object()
                    .ok_or_else(|| error("Proxy target must be an object"))?;
                let handler = arg(1);
                if handler.object().is_none() {
                    return Err(error("Proxy handler must be an object"));
                }
                let callable = self.callable(&JsValue::Object(target));
                let constructible = match &self.heap.get(target)?.kind {
                    ObjectKind::Function(f) => f.constructable,
                    ObjectKind::Native(_) | ObjectKind::Bound { .. } => true,
                    ObjectKind::Proxy(p) => p.constructible,
                    _ => false,
                };
                let proxy = JsValue::Object(self.allocate(
                    ObjectKind::Proxy(Box::new(ProxyData {
                        target,
                        handler: Some(handler),
                        callable,
                        constructible,
                    })),
                    None,
                )?);
                if name == "Proxy" {
                    return Ok(proxy);
                }
                let revoke = JsValue::Object(self.allocate(
                    ObjectKind::ProxyRevoker(
                        proxy.object().ok_or_else(|| error("Proxy allocation"))?,
                    ),
                    Some(self.function_proto),
                )?);
                let object = self.object()?;
                self.set(object.clone(), "proxy", proxy)?;
                self.set(object.clone(), "revoke", revoke)?;
                Ok(object)
            })());
        }
        let method = name.strip_prefix("Reflect.")?;
        Some((|| {
            let target = arg(0);
            if method != "apply" && method != "construct" && target.object().is_none() {
                return Err(error("Reflect target must be an object"));
            }
            match method {
                "get" => {
                    let key = self.property_key(arg(1))?;
                    self.get_key_receiver(
                        target.clone(),
                        key,
                        if args.len() > 2 { arg(2) } else { target },
                    )
                }
                "set" => {
                    let key = self.property_key(arg(1))?;
                    Ok(JsValue::Bool(self.reflect_set(
                        target.clone(),
                        key,
                        arg(2),
                        if args.len() > 3 { arg(3) } else { target },
                    )?))
                }
                "has" => {
                    let key = self.property_key(arg(1))?;
                    Ok(JsValue::Bool(self.has(target, key)?))
                }
                "deleteProperty" => {
                    let key = self.property_key(arg(1))?;
                    Ok(JsValue::Bool(self.delete(target, key)?))
                }
                "ownKeys" => {
                    let keys = self.keys(&target, false)?;
                    self.array(keys.iter().map(key_value).collect())
                }
                "getOwnPropertyDescriptor" => {
                    let key = self.property_key(arg(1))?;
                    if let Some(desc) = self.own_descriptor(target, &key)? {
                        self.descriptor_object(desc)
                    } else {
                        Ok(JsValue::Undefined)
                    }
                }
                "defineProperty" => {
                    let key = self.property_key(arg(1))?;
                    let descriptor = self.read_descriptor(arg(2))?;
                    Ok(JsValue::Bool(
                        self.reflect_define_partial(target, key, descriptor)?,
                    ))
                }
                "getPrototypeOf" => self.object_prototype(target),
                "setPrototypeOf" => Ok(JsValue::Bool(self.set_object_prototype(target, arg(1))?)),
                "isExtensible" => Ok(JsValue::Bool(self.object_extensible(target)?)),
                "preventExtensions" => Ok(JsValue::Bool(self.prevent_extensions(target)?)),
                "apply" => {
                    let values = self.array_values(arg(2))?;
                    self.call_inner(target, arg(1), values, false, None)
                }
                "construct" => {
                    let values = self.array_values(arg(1))?;
                    let object = self.construct(target.clone(), values)?;
                    if args.len() > 2 && arg(2) != target {
                        let constructor = arg(2);
                        if !self.callable(&constructor) {
                            return Err(error("newTarget constructor"));
                        }
                        let proto = self.get(constructor, "prototype")?;
                        if let Some(id) = object.object() {
                            self.heap.get_mut(id)?.prototype =
                                proto.object().or(Some(self.object_proto));
                        }
                    }
                    Ok(object)
                }
                _ => Err(error("Reflect method")),
            }
        })())
    }
}
