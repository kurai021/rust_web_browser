use super::*;
impl Vm {
    pub(super) fn install_symbols(&mut self) -> Result<(), JsError> {
        self.symbol_proto = self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?;
        let constructor = self.get_global("Symbol")?;
        if let Some(id) = constructor.object() {
            self.define(
                id,
                "prototype".into(),
                Property {
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    ..Property::data(JsValue::Object(self.symbol_proto))
                },
            )?;
        }
        for name in ["toString", "valueOf"] {
            let value = self.native(&format!("Symbol.prototype.{name}"))?;
            self.define(
                self.symbol_proto,
                name.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        let primitive = self.native("Symbol.prototype.valueOf")?;
        self.define(
            self.symbol_proto,
            PropertyKey::Symbol(crate::symbols::TO_PRIMITIVE),
            Property {
                writable: false,
                enumerable: false,
                configurable: true,
                ..Property::data(primitive)
            },
        )?;
        self.define(
            self.symbol_proto,
            PropertyKey::Symbol(crate::symbols::TO_STRING_TAG),
            Property {
                writable: false,
                enumerable: false,
                configurable: true,
                ..Property::data(JsValue::string("Symbol"))
            },
        )?;
        self.define(
            self.symbol_proto,
            "constructor".into(),
            Property {
                enumerable: false,
                ..Property::data(constructor)
            },
        )?;
        let getter = self.native("Symbol.description")?;
        self.define(
            self.symbol_proto,
            "description".into(),
            Property {
                value: PropertyValue::Accessor {
                    get: getter,
                    set: JsValue::Undefined,
                },
                writable: false,
                enumerable: false,
                configurable: true,
            },
        )?;
        let instance = self.native("Function.hasInstance")?;
        self.define(
            self.function_proto,
            PropertyKey::Symbol(crate::symbols::HAS_INSTANCE),
            Property {
                writable: false,
                enumerable: false,
                configurable: false,
                ..Property::data(instance)
            },
        )?;
        Ok(())
    }
    pub(crate) fn primitive_prototype(&self, value: &JsValue) -> ObjectId {
        match value {
            JsValue::String(_) => self.string_proto,
            JsValue::Number(_) => self.number_proto,
            JsValue::Bool(_) => self.boolean_proto,
            JsValue::BigInt(_) => self.bigint_proto,
            JsValue::Symbol(..) => self.symbol_proto,
            _ => self.object_proto,
        }
    }
    pub(crate) fn symbol_value(&self, id: u64) -> JsValue {
        let description = self
            .symbol_descriptions
            .get(&id)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_default();
        JsValue::Symbol(id, description.into())
    }
    pub(crate) fn symbol_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
        construct: bool,
    ) -> Option<Result<JsValue, JsError>> {
        if name == "Function.hasInstance" {
            return Some(
                self.ordinary_instanceof(args.first().cloned().unwrap_or(JsValue::Undefined), this)
                    .map(JsValue::Bool),
            );
        }
        if name != "Symbol" && !name.starts_with("Symbol.") {
            return None;
        }
        let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
        Some((|| match name {
            "Symbol" | "Symbol.for" => {
                if construct {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "Symbol is not a constructor",
                    ));
                }
                let description = if arg == JsValue::Undefined && name == "Symbol" {
                    None
                } else {
                    Some(self.to_string(arg)?)
                };
                if name == "Symbol.for" {
                    if let Some(value) = self
                        .symbol_registry
                        .get(description.as_deref().unwrap_or(""))
                    {
                        return Ok(value.clone());
                    }
                    if self.symbol_registry.len() >= 100_000 {
                        return Err(JsError::new(
                            ErrorKind::MemoryLimit,
                            "symbol registry quota",
                        ));
                    }
                }
                self.reserve_memory(
                    description
                        .as_ref()
                        .map_or(128, |s| s.len().saturating_mul(3) + 224),
                )?;
                let id = self.symbol_next;
                self.symbol_next = self
                    .symbol_next
                    .checked_add(1)
                    .ok_or_else(|| JsError::new(ErrorKind::MemoryLimit, "symbol identity quota"))?;
                let value = JsValue::Symbol(id, description.clone().unwrap_or_default().into());
                self.symbol_descriptions.insert(id, description.clone());
                if name == "Symbol.for" {
                    self.registered_symbols.insert(id);
                    self.symbol_registry
                        .insert(description.unwrap_or_default(), value.clone());
                }
                Ok(value)
            }
            "Symbol.keyFor" => {
                let JsValue::Symbol(id, _) = arg else {
                    return Err(JsError::new(ErrorKind::TypeError, "Symbol.keyFor argument"));
                };
                Ok(self
                    .symbol_registry
                    .iter()
                    .find_map(|(key, value)| {
                        matches!(value,JsValue::Symbol(other,_)if *other==id)
                            .then(|| JsValue::string(key))
                    })
                    .unwrap_or(JsValue::Undefined))
            }
            _ => {
                let value = if let JsValue::Object(id) = &this {
                    match &self.heap.get(*id)?.kind {
                        ObjectKind::Boxed(v) => v.clone(),
                        _ => this,
                    }
                } else {
                    this
                };
                let JsValue::Symbol(id, description) = value else {
                    return Err(JsError::new(ErrorKind::TypeError, "Symbol receiver"));
                };
                let description = self
                    .symbol_descriptions
                    .get(&id)
                    .and_then(Option::as_ref)
                    .map_or(description.clone(), |s| Arc::from(s.as_str()));
                match name {
                    "Symbol.prototype.valueOf" => Ok(JsValue::Symbol(id, description)),
                    "Symbol.description" => Ok(self
                        .symbol_descriptions
                        .get(&id)
                        .map_or_else(|| Some(description.to_string()), Clone::clone)
                        .map(JsValue::string)
                        .unwrap_or(JsValue::Undefined)),
                    "Symbol.prototype.toString" => {
                        Ok(JsValue::string(format!("Symbol({description})")))
                    }
                    _ => Err(JsError::new(ErrorKind::TypeError, "Symbol intrinsic")),
                }
            }
        })())
    }
    pub(crate) fn ordinary_instanceof(
        &mut self,
        left: JsValue,
        right: JsValue,
    ) -> Result<bool, JsError> {
        if !self.callable(&right) {
            return Ok(false);
        }
        if let Some(id) = right.object() {
            if let ObjectKind::Bound { function, .. } = self.heap.get(id)?.kind.clone() {
                return self
                    .binary("instanceof", left, function)
                    .map(|v| v.truthy());
            }
        }
        let Some(mut current) = left.object() else {
            return Ok(false);
        };
        let prototype = self
            .get(right, "prototype")?
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "instanceof prototype"))?;
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(next) = self.object_prototype(JsValue::Object(current))?.object() else {
                return Ok(false);
            };
            if next == prototype {
                return Ok(true);
            }
            current = next;
        }
        Ok(false)
    }
}
