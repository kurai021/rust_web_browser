//! Bounded data-only graphs for messages/history/storage; never shares VM IDs.
use crate::{value::*, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CloneValue {
    BigInt(String),
    Undefined,
    Null,
    Bool(bool),
    Number(u64),
    String(String),
    Ref(usize),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneNode {
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub entries: Vec<(CloneValue, CloneValue)>,
    #[serde(default)]
    pub regexp: Option<(String, String)>,
    #[serde(default)]
    pub boxed: Option<CloneValue>,
    #[serde(default)]
    pub error: bool,
    pub array: bool,
    pub length: usize,
    pub date: Option<u64>,
    pub properties: Vec<(String, CloneValue)>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneData {
    pub root: CloneValue,
    pub nodes: Vec<CloneNode>,
}
impl CloneData {
    pub fn retained_bytes(&self) -> usize {
        fn value(v: &CloneValue) -> usize {
            match v {
                CloneValue::String(s) | CloneValue::BigInt(s) => s.len() + 32,
                _ => 32,
            }
        }
        value(&self.root)
            + self
                .nodes
                .iter()
                .map(|node| {
                    192 + node.collection.as_ref().map_or(0, String::len)
                        + node
                            .regexp
                            .as_ref()
                            .map_or(0, |(source, flags)| source.len() + flags.len())
                        + node.boxed.as_ref().map_or(0, value)
                        + node
                            .entries
                            .iter()
                            .map(|(key, v)| 64 + value(key) + value(v))
                            .sum::<usize>()
                        + node
                            .properties
                            .iter()
                            .map(|(key, v)| key.len() + 64 + value(v))
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}
impl Vm {
    pub fn serialize_clone(&mut self, input: JsValue) -> Result<CloneData, JsError> {
        fn value(
            vm: &mut Vm,
            v: JsValue,
            map: &mut HashMap<ObjectId, usize>,
            nodes: &mut Vec<CloneNode>,
            work: &mut Vec<ObjectId>,
            bytes: &mut usize,
        ) -> Result<CloneValue, JsError> {
            vm.checkpoint()?;
            Ok(match v {
                JsValue::BigInt(value) => {
                    let value = value.to_radix(10)?;
                    *bytes = bytes.saturating_add(value.len());
                    vm.check_string_bytes(*bytes)?;
                    CloneValue::BigInt(value)
                }
                JsValue::Undefined => CloneValue::Undefined,
                JsValue::Null => CloneValue::Null,
                JsValue::Bool(b) => CloneValue::Bool(b),
                JsValue::Number(n) => CloneValue::Number(n.to_bits()),
                JsValue::String(s) => {
                    *bytes = bytes.saturating_add(s.len());
                    vm.check_string_bytes(*bytes)?;
                    CloneValue::String(s.to_string())
                }
                JsValue::Symbol(..) => {
                    return Err(JsError::new(ErrorKind::TypeError, "DataCloneError: Symbol"))
                }
                JsValue::Object(id) => {
                    if let Some(i) = map.get(&id) {
                        return Ok(CloneValue::Ref(*i));
                    }
                    let kind = &vm.heap.get(id)?.kind;
                    let (array, date) = match kind {
                        ObjectKind::Ordinary | ObjectKind::Error => (false, None),
                        ObjectKind::Array => (true, None),
                        ObjectKind::Date(n) => (false, Some(n.to_bits())),
                        ObjectKind::RegExp { .. } | ObjectKind::Boxed(_) => (false, None),
                        ObjectKind::Collection(data)
                            if data.kind != crate::vm::collections::CollectionKind::WeakMap =>
                        {
                            (false, None)
                        }
                        _ => {
                            return Err(JsError::new(
                                ErrorKind::TypeError,
                                "DataCloneError: unsupported object",
                            ))
                        }
                    };
                    if nodes.len() >= 100_000 {
                        return Err(JsError::new(ErrorKind::MemoryLimit, "clone node quota"));
                    }
                    let index = nodes.len();
                    map.insert(id, index);
                    nodes.push(CloneNode {
                        collection: None,
                        entries: Vec::new(),
                        regexp: None,
                        boxed: None,
                        error: false,
                        array,
                        length: 0,
                        date,
                        properties: Vec::new(),
                    });
                    work.push(id);
                    CloneValue::Ref(index)
                }
            })
        }
        let mut map = HashMap::new();
        let mut nodes = Vec::new();
        let mut work = Vec::new();
        let mut bytes = 0;
        let root = value(self, input, &mut map, &mut nodes, &mut work, &mut bytes)?;
        while let Some(id) = work.pop() {
            let index = map[&id];
            match self.heap.get(id)?.kind.clone() {
                ObjectKind::Collection(data) => {
                    nodes[index].collection = Some(
                        if data.kind == crate::vm::collections::CollectionKind::Map {
                            "Map"
                        } else {
                            "Set"
                        }
                        .into(),
                    );
                    let mut entries = Vec::new();
                    for (k, v) in data.entries.values() {
                        let k =
                            value(self, k.clone(), &mut map, &mut nodes, &mut work, &mut bytes)?;
                        let v =
                            value(self, v.clone(), &mut map, &mut nodes, &mut work, &mut bytes)?;
                        entries.push((k, v));
                    }
                    nodes[index].entries = entries;
                    continue;
                }
                ObjectKind::RegExp { source, flags, .. } => {
                    bytes = bytes.saturating_add(source.len() + flags.len());
                    self.check_string_bytes(bytes)?;
                    nodes[index].regexp = Some((source, flags));
                    continue;
                }
                ObjectKind::Boxed(boxed) => {
                    let boxed = value(self, boxed, &mut map, &mut nodes, &mut work, &mut bytes)?;
                    nodes[index].boxed = Some(boxed);
                    continue;
                }
                ObjectKind::Error => {
                    nodes[index].error = true;
                    let mut properties = Vec::new();
                    for key in ["name", "message", "stack", "cause"] {
                        let property = self.get(JsValue::Object(id), key)?;
                        if property != JsValue::Undefined {
                            let property =
                                value(self, property, &mut map, &mut nodes, &mut work, &mut bytes)?;
                            properties.push((key.into(), property));
                        }
                    }
                    nodes[index].properties = properties;
                    continue;
                }
                _ => {}
            }
            let mut properties = Vec::new();
            for key in self.keys(&JsValue::Object(id), true)? {
                let PropertyKey::String(name) = key else {
                    continue;
                };
                bytes = bytes.saturating_add(name.len() + 64);
                self.check_string_bytes(bytes)?;
                let v = self.get(JsValue::Object(id), &name)?;
                let v = value(self, v, &mut map, &mut nodes, &mut work, &mut bytes)?;
                properties.push((name, v));
            }
            let length = if nodes[index].array {
                self.array_length(JsValue::Object(id))?
            } else {
                0
            };
            nodes[index].properties = properties;
            nodes[index].length = length;
        }
        Ok(CloneData { root, nodes })
    }
    pub fn deserialize_clone(&mut self, data: &CloneData) -> Result<JsValue, JsError> {
        if data.nodes.len() > 100_000 {
            return Err(JsError::new(ErrorKind::MemoryLimit, "clone node quota"));
        }
        let mut bytes = 0usize;
        fn validate(value: &CloneValue, count: usize, bytes: &mut usize) -> Result<(), JsError> {
            match value {
                CloneValue::String(s) | CloneValue::BigInt(s) => {
                    *bytes = bytes.saturating_add(s.len())
                }
                CloneValue::Ref(i) if *i >= count => {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "DataCloneError: invalid reference",
                    ))
                }
                _ => {}
            }
            Ok(())
        }
        validate(&data.root, data.nodes.len(), &mut bytes)?;
        for node in &data.nodes {
            self.checkpoint()?;
            if node
                .collection
                .as_ref()
                .is_some_and(|kind| kind != "Map" && kind != "Set")
                || node.boxed.as_ref().is_some_and(|value| {
                    !matches!(
                        value,
                        CloneValue::Bool(_)
                            | CloneValue::String(_)
                            | CloneValue::Number(_)
                            | CloneValue::BigInt(_)
                    )
                })
                || node.entries.len() > 100_000
            {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "DataCloneError: invalid built-in payload",
                ));
            }
            let tags = usize::from(node.array)
                + usize::from(node.date.is_some())
                + usize::from(node.collection.is_some())
                + usize::from(node.regexp.is_some())
                + usize::from(node.boxed.is_some())
                + usize::from(node.error);
            if tags > 1 || node.collection.is_none() && !node.entries.is_empty() {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "DataCloneError: conflicting node tags",
                ));
            }
            if node.length > u32::MAX as usize {
                return Err(JsError::new(ErrorKind::MemoryLimit, "clone array length"));
            }
            for (name, value) in &node.properties {
                bytes = bytes.saturating_add(name.len() + 64);
                validate(value, data.nodes.len(), &mut bytes)?;
                self.check_string_bytes(bytes)?;
            }
            for (k, v) in &node.entries {
                bytes = bytes.saturating_add(128);
                validate(k, data.nodes.len(), &mut bytes)?;
                validate(v, data.nodes.len(), &mut bytes)?;
            }
            if let Some(value) = &node.boxed {
                validate(value, data.nodes.len(), &mut bytes)?;
            }
            if let Some((source, flags)) = &node.regexp {
                bytes = bytes.saturating_add(source.len() + flags.len());
            }
            self.check_string_bytes(bytes)?;
        }
        self.check_string_bytes(bytes)?;
        let mut objects = Vec::new();
        for node in &data.nodes {
            let object = if let Some(collection) = &node.collection {
                self.collection_for_clone(collection)?
            } else if let Some((source, flags)) = &node.regexp {
                self.regexp_for_clone(source, flags)?
            } else if node.boxed.is_some() {
                self.box_for_clone(JsValue::Undefined)?
            } else if node.error {
                self.error_value(JsError::new(ErrorKind::Error, ""))?
            } else if let Some(bits) = node.date {
                JsValue::Object(self.allocate(
                    ObjectKind::Date(f64::from_bits(bits)),
                    Some(self.date_proto),
                )?)
            } else if node.array {
                self.array(Vec::new())?
            } else {
                self.object()?
            };
            objects.push(object);
        }
        fn value(v: &CloneValue, objects: &[JsValue]) -> Result<JsValue, JsError> {
            Ok(match v {
                CloneValue::BigInt(value) => {
                    JsValue::BigInt(std::sync::Arc::new(crate::bigint::BigInt::parse(value)?))
                }
                CloneValue::Undefined => JsValue::Undefined,
                CloneValue::Null => JsValue::Null,
                CloneValue::Bool(b) => JsValue::Bool(*b),
                CloneValue::Number(n) => JsValue::Number(f64::from_bits(*n)),
                CloneValue::String(s) => JsValue::string(s),
                CloneValue::Ref(i) => objects.get(*i).cloned().ok_or_else(|| {
                    JsError::new(ErrorKind::TypeError, "DataCloneError: invalid reference")
                })?,
            })
        }
        for (i, node) in data.nodes.iter().enumerate() {
            if node.collection.is_some() {
                for (k, v) in &node.entries {
                    self.collection_insert(
                        objects[i].object().ok_or_else(|| {
                            JsError::new(ErrorKind::TypeError, "clone collection")
                        })?,
                        value(k, &objects)?,
                        value(v, &objects)?,
                    )?;
                }
                continue;
            }
            if let Some(boxed) = &node.boxed {
                let boxed = value(boxed, &objects)?;
                self.fill_box_for_clone(&objects[i], boxed)?;
                continue;
            }
            for (name, v) in &node.properties {
                self.checkpoint()?;
                self.define(
                    objects[i].object().unwrap(),
                    name.as_str().into(),
                    Property::data(value(v, &objects)?),
                )?;
            }
            if node.array {
                self.set(
                    objects[i].clone(),
                    "length",
                    JsValue::Number(node.length as f64),
                )?;
            }
        }
        value(&data.root, &objects)
    }
}
