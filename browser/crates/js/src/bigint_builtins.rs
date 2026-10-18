use super::*;
use crate::bigint::BigInt;
impl Vm {
    pub(super) fn install_bigint(&mut self) -> Result<(), JsError> {
        self.bigint_proto = self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?;
        let constructor = self.native("BigInt")?;
        if let Some(id) = constructor.object() {
            self.define(
                id,
                "prototype".into(),
                Property {
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    ..Property::data(JsValue::Object(self.bigint_proto))
                },
            )?;
            for method in ["asIntN", "asUintN"] {
                let value = self.native(&format!("BigInt.{method}"))?;
                self.define(
                    id,
                    method.into(),
                    Property {
                        enumerable: false,
                        ..Property::data(value)
                    },
                )?;
            }
        }
        self.define(
            self.bigint_proto,
            "constructor".into(),
            Property {
                enumerable: false,
                ..Property::data(constructor.clone())
            },
        )?;
        for method in ["toString", "toLocaleString", "valueOf"] {
            let value = self.native(&format!("BigInt.prototype.{method}"))?;
            self.define(
                self.bigint_proto,
                method.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        self.define(
            self.bigint_proto,
            PropertyKey::Symbol(crate::symbols::TO_STRING_TAG),
            Property {
                writable: false,
                enumerable: false,
                configurable: true,
                ..Property::data(JsValue::string("BigInt"))
            },
        )?;
        self.set_global("BigInt", constructor)
    }
    pub(crate) fn bigint_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
        construct: bool,
    ) -> Option<Result<JsValue, JsError>> {
        if name != "BigInt" && !name.starts_with("BigInt.") {
            return None;
        }
        let arg = |i| args.get(i).cloned().unwrap_or(JsValue::Undefined);
        Some((|| match name {
            "BigInt" => {
                if construct {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "BigInt is not a constructor",
                    ));
                }
                let value = arg(0);
                let value = if value.object().is_some() {
                    self.coerce_primitive(value, true)?
                } else {
                    value
                };
                let bigint = match value {
                    JsValue::BigInt(value) => return Ok(JsValue::BigInt(value)),
                    JsValue::Bool(value) => {
                        if value {
                            BigInt::one()
                        } else {
                            BigInt::zero()
                        }
                    }
                    JsValue::Number(value) => BigInt::from_f64(value)?,
                    JsValue::String(value) => BigInt::parse(&value)?,
                    _ => {
                        return Err(JsError::new(
                            ErrorKind::TypeError,
                            "invalid BigInt conversion",
                        ))
                    }
                };
                Ok(JsValue::BigInt(Arc::new(bigint)))
            }
            "BigInt.asIntN" | "BigInt.asUintN" => {
                let bits = self.to_number(arg(0))?;
                if !bits.is_finite() || bits < 0.0 || bits > 16384.0 {
                    return Err(JsError::new(ErrorKind::RangeError, "BigInt width quota"));
                }
                let value = arg(1);
                let value = if value.object().is_some() {
                    self.coerce_primitive(value, true)?
                } else {
                    value
                };
                let value = match value {
                    JsValue::BigInt(value) => value,
                    JsValue::Bool(value) => {
                        Arc::new(if value { BigInt::one() } else { BigInt::zero() })
                    }
                    JsValue::String(value) => Arc::new(BigInt::parse(&value)?),
                    _ => return Err(JsError::new(ErrorKind::TypeError, "BigInt required")),
                };
                Ok(JsValue::BigInt(Arc::new(value.as_bits(
                    bits.trunc() as usize,
                    name == "BigInt.asIntN",
                )?)))
            }
            _ => {
                let value = match this {
                    JsValue::BigInt(value) => value,
                    JsValue::Object(id) => match &self.heap.get(id)?.kind {
                        ObjectKind::Boxed(JsValue::BigInt(value)) => value.clone(),
                        _ => return Err(JsError::new(ErrorKind::TypeError, "BigInt receiver")),
                    },
                    _ => return Err(JsError::new(ErrorKind::TypeError, "BigInt receiver")),
                };
                if name == "BigInt.prototype.valueOf" {
                    return Ok(JsValue::BigInt(value));
                }
                let radix = if arg(0) == JsValue::Undefined {
                    10
                } else {
                    self.to_number(arg(0))? as u32
                };
                Ok(JsValue::string(value.to_radix(radix)?))
            }
        })())
    }
}
