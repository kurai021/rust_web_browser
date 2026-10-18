//! Presence-aware descriptors: omitted fields retain their existing attributes,
//! while Proxy traps receive exactly the fields supplied by the operation.
use super::*;
#[derive(Debug, Clone, Default)]
pub(crate) struct Descriptor {
    pub value: Option<JsValue>,
    pub get: Option<JsValue>,
    pub set: Option<JsValue>,
    pub writable: Option<bool>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}
impl Descriptor {
    pub fn accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }
    pub fn data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }
    pub fn complete(&self, current: Option<&Property>) -> Property {
        let mut result = current.cloned().unwrap_or(Property {
            value: PropertyValue::Data(JsValue::Undefined),
            writable: false,
            enumerable: false,
            configurable: false,
        });
        if self.accessor() {
            let (get, set) = match &result.value {
                PropertyValue::Accessor { get, set } => (get.clone(), set.clone()),
                _ => (JsValue::Undefined, JsValue::Undefined),
            };
            result.value = PropertyValue::Accessor {
                get: self.get.clone().unwrap_or(get),
                set: self.set.clone().unwrap_or(set),
            };
            result.writable = false;
        } else if self.data() {
            let value = match &result.value {
                PropertyValue::Data(value) => value.clone(),
                _ => {
                    result.writable = false;
                    JsValue::Undefined
                }
            };
            result.value = PropertyValue::Data(self.value.clone().unwrap_or(value));
            if let Some(writable) = self.writable {
                result.writable = writable;
            }
        }
        if let Some(value) = self.enumerable {
            result.enumerable = value;
        }
        if let Some(value) = self.configurable {
            result.configurable = value;
        }
        result
    }
    pub fn full(property: Property) -> Self {
        let mut out = Self {
            writable: None,
            enumerable: Some(property.enumerable),
            configurable: Some(property.configurable),
            ..Self::default()
        };
        match property.value {
            PropertyValue::Data(value) => {
                out.value = Some(value);
                out.writable = Some(property.writable);
            }
            PropertyValue::Accessor { get, set } => {
                out.get = Some(get);
                out.set = Some(set);
            }
        }
        out
    }
}
impl Vm {
    pub(crate) fn read_descriptor(&mut self, input: JsValue) -> Result<Descriptor, JsError> {
        if input.object().is_none() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "property descriptor must be object",
            ));
        }
        let mut out = Descriptor::default();
        for key in [
            "enumerable",
            "configurable",
            "value",
            "writable",
            "get",
            "set",
        ] {
            if !self.has(input.clone(), key.into())? {
                continue;
            }
            let value = self.get(input.clone(), key)?;
            match key {
                "enumerable" => out.enumerable = Some(value.truthy()),
                "configurable" => out.configurable = Some(value.truthy()),
                "writable" => out.writable = Some(value.truthy()),
                "value" => out.value = Some(value),
                _ => {
                    if value != JsValue::Undefined && !self.callable(&value) {
                        return Err(JsError::new(ErrorKind::TypeError, "invalid accessor"));
                    }
                    if key == "get" {
                        out.get = Some(value);
                    } else {
                        out.set = Some(value);
                    }
                }
            }
        }
        if out.accessor() && out.data() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "mixed property descriptor",
            ));
        }
        Ok(out)
    }
    pub(crate) fn partial_descriptor_object(
        &mut self,
        descriptor: &Descriptor,
    ) -> Result<JsValue, JsError> {
        let object = self.object()?;
        for (name, value) in [
            ("value", descriptor.value.clone()),
            ("writable", descriptor.writable.map(JsValue::Bool)),
            ("get", descriptor.get.clone()),
            ("set", descriptor.set.clone()),
            ("enumerable", descriptor.enumerable.map(JsValue::Bool)),
            ("configurable", descriptor.configurable.map(JsValue::Bool)),
        ] {
            if let Some(value) = value {
                self.set(object.clone(), name, value)?;
            }
        }
        Ok(object)
    }
}
