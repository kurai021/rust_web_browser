//! Ordered strong collections and ephemeron weak maps over generational IDs.
use super::*;
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionKind {
    Map,
    Set,
    WeakMap,
}
impl CollectionKind {
    fn index(self) -> usize {
        match self {
            Self::Map => 0,
            Self::Set => 1,
            Self::WeakMap => 2,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Map => "Map",
            Self::Set => "Set",
            Self::WeakMap => "WeakMap",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Undefined,
    Null,
    Bool(bool),
    Number(u64),
    String(Arc<str>),
    BigInt(Arc<crate::bigint::BigInt>),
    Symbol(u64),
    Object(ObjectId),
}
fn key(value: &JsValue) -> Key {
    match value {
        JsValue::Undefined => Key::Undefined,
        JsValue::Null => Key::Null,
        JsValue::Bool(v) => Key::Bool(*v),
        JsValue::Number(v) => Key::Number(if *v == 0.0 {
            0
        } else if v.is_nan() {
            f64::NAN.to_bits()
        } else {
            v.to_bits()
        }),
        JsValue::String(v) => Key::String(v.clone()),
        JsValue::BigInt(v) => Key::BigInt(v.clone()),
        JsValue::Symbol(v, _) => Key::Symbol(*v),
        JsValue::Object(v) => Key::Object(*v),
    }
}
#[derive(Debug, Clone)]
pub(crate) struct CollectionData {
    pub kind: CollectionKind,
    pub entries: BTreeMap<u64, (JsValue, JsValue)>,
    lookup: HashMap<Key, u64>,
    next: u64,
}
impl CollectionData {
    pub fn new(kind: CollectionKind) -> Self {
        Self {
            kind,
            entries: BTreeMap::new(),
            lookup: HashMap::new(),
            next: 0,
        }
    }
    pub fn roots(&self) -> Vec<JsValue> {
        if self.kind == CollectionKind::WeakMap {
            Vec::new()
        } else {
            self.entries
                .values()
                .flat_map(|(k, v)| [k.clone(), v.clone()])
                .collect()
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.entries
            .values()
            .map(|(k, v)| 192 + k.retained_bytes() + v.retained_bytes())
            .sum()
    }
    pub fn next_after(&self, after: Option<u64>) -> Option<(u64, JsValue, JsValue)> {
        let range = if let Some(after) = after {
            self.entries
                .range((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded))
        } else {
            self.entries.range(..)
        };
        range
            .into_iter()
            .next()
            .map(|(&i, (k, v))| (i, k.clone(), v.clone()))
    }
    pub fn sweep_weak(&mut self, marked: &HashSet<ObjectId>, symbols: &HashSet<u64>) {
        if self.kind == CollectionKind::WeakMap {
            self.entries.retain(|_, (key, _)| match key {
                JsValue::Object(id) => marked.contains(id),
                JsValue::Symbol(id, _) => symbols.contains(id),
                _ => false,
            });
            self.lookup
                .retain(|_, index| self.entries.contains_key(index));
        }
    }
}
impl Vm {
    pub(crate) fn collection_for_clone(&mut self, name: &str) -> Result<JsValue, JsError> {
        let kind = match name {
            "Map" => CollectionKind::Map,
            "Set" => CollectionKind::Set,
            _ => {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "DataCloneError: collection tag",
                ))
            }
        };
        Ok(JsValue::Object(self.allocate(
            ObjectKind::Collection(Box::new(CollectionData::new(kind))),
            Some(self.collection_protos[kind.index()]),
        )?))
    }
    pub(super) fn install_collections(&mut self) -> Result<(), JsError> {
        for kind in [
            CollectionKind::Map,
            CollectionKind::Set,
            CollectionKind::WeakMap,
        ] {
            let proto = self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?;
            self.collection_protos[kind.index()] = proto;
            let name = kind.name();
            let constructor = self.native(name)?;
            let id = constructor
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::Error, "collection intrinsic"))?;
            self.define(
                id,
                "prototype".into(),
                Property {
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    ..Property::data(JsValue::Object(proto))
                },
            )?;
            self.define(
                proto,
                "constructor".into(),
                Property {
                    enumerable: false,
                    ..Property::data(constructor.clone())
                },
            )?;
            let methods = match kind {
                CollectionKind::Map => vec![
                    "get", "set", "has", "delete", "clear", "keys", "values", "entries", "forEach",
                ],
                CollectionKind::Set => vec![
                    "add", "has", "delete", "clear", "keys", "values", "entries", "forEach",
                ],
                CollectionKind::WeakMap => vec!["get", "set", "has", "delete"],
            };
            for method in methods {
                let value = self.native(&format!("{name}.prototype.{method}"))?;
                self.define(
                    proto,
                    method.into(),
                    Property {
                        enumerable: false,
                        ..Property::data(value)
                    },
                )?;
            }
            self.define(
                proto,
                PropertyKey::Symbol(crate::symbols::TO_STRING_TAG),
                Property {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    ..Property::data(JsValue::string(name))
                },
            )?;
            if kind != CollectionKind::WeakMap {
                let size = self.native(&format!("{name}.size"))?;
                self.define(
                    proto,
                    "size".into(),
                    Property {
                        value: PropertyValue::Accessor {
                            get: size,
                            set: JsValue::Undefined,
                        },
                        writable: false,
                        enumerable: false,
                        configurable: true,
                    },
                )?;
                let default = self.get(
                    JsValue::Object(proto),
                    if kind == CollectionKind::Map {
                        "entries"
                    } else {
                        "values"
                    },
                )?;
                self.define(
                    proto,
                    PropertyKey::Symbol(crate::symbols::ITERATOR),
                    Property {
                        enumerable: false,
                        ..Property::data(default)
                    },
                )?;
                if kind == CollectionKind::Set {
                    let values = self.get(JsValue::Object(proto), "values")?;
                    self.define(
                        proto,
                        "keys".into(),
                        Property {
                            enumerable: false,
                            ..Property::data(values)
                        },
                    )?;
                }
            }
            self.set_global(name, constructor)?;
        }
        let clone = self.native("structuredClone")?;
        self.set_global("structuredClone", clone)?;
        Ok(())
    }
    pub(crate) fn collection_next(
        &self,
        id: ObjectId,
        after: Option<u64>,
    ) -> Result<Option<(u64, JsValue, JsValue)>, JsError> {
        let ObjectKind::Collection(data) = &self.heap.get(id)?.kind else {
            return Err(JsError::new(ErrorKind::TypeError, "collection iterator"));
        };
        Ok(data.next_after(after))
    }
    pub(crate) fn collection_insert(
        &mut self,
        id: ObjectId,
        k: JsValue,
        v: JsValue,
    ) -> Result<(), JsError> {
        let ObjectKind::Collection(data) = &self.heap.get(id)?.kind else {
            return Err(JsError::new(ErrorKind::TypeError, "collection receiver"));
        };
        if data.kind == CollectionKind::WeakMap
            && k.object().is_none()
            && !matches!(&k,JsValue::Symbol(id,_) if !self.registered_symbols.contains(id))
        {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "WeakMap key must be an object or unregistered Symbol",
            ));
        }
        if data.entries.len() >= 100_000 && !data.lookup.contains_key(&key(&k)) {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "collection entry quota",
            ));
        }
        self.reserve_memory(192 + k.retained_bytes() + v.retained_bytes())?;
        let ObjectKind::Collection(data) = &mut self.heap.get_mut(id)?.kind else {
            return Err(JsError::new(ErrorKind::TypeError, "collection receiver"));
        };
        let hash = key(&k);
        if let Some(index) = data.lookup.get(&hash) {
            if let Some((_, value)) = data.entries.get_mut(index) {
                *value = v;
            }
        } else {
            data.next = data
                .next
                .checked_add(1)
                .ok_or_else(|| JsError::new(ErrorKind::MemoryLimit, "collection sequence quota"))?;
            let k = if matches!(k,JsValue::Number(n)if n==0.0) {
                JsValue::Number(0.0)
            } else {
                k
            };
            data.lookup.insert(hash, data.next);
            data.entries.insert(data.next, (k, v));
        }
        Ok(())
    }
    pub(crate) fn collection_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
        construct: bool,
    ) -> Option<Result<JsValue, JsError>> {
        if name == "structuredClone" {
            return Some((|| {
                let input = args.first().cloned().unwrap_or(JsValue::Undefined);
                let data = self.serialize_clone(input).map_err(|error| {
                    if error.is_resource_limit() {
                        error
                    } else {
                        let mut error = self
                            .error_value(error)
                            .and_then(|v| {
                                self.set(v.clone(), "name", JsValue::string("DataCloneError"))?;
                                Ok(v)
                            })
                            .map(|v| JsError {
                                kind: ErrorKind::Error,
                                message: "DataCloneError".into(),
                                line: 0,
                                column: 0,
                                stack: Vec::new(),
                                thrown: Some(v),
                            })
                            .unwrap_or_else(|e| e);
                        error.message = "DataCloneError".into();
                        error
                    }
                })?;
                self.deserialize_clone(&data)
            })());
        }
        let kind = if name == "Map" || name.starts_with("Map.") {
            CollectionKind::Map
        } else if name == "Set" || name.starts_with("Set.") {
            CollectionKind::Set
        } else if name == "WeakMap" || name.starts_with("WeakMap.") {
            CollectionKind::WeakMap
        } else {
            return None;
        };
        let arg = |i| args.get(i).cloned().unwrap_or(JsValue::Undefined);
        Some((|| {
            if name == kind.name() {
                if !construct {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "collection requires new",
                    ));
                }
                let value = JsValue::Object(self.allocate(
                    ObjectKind::Collection(Box::new(CollectionData::new(kind))),
                    Some(self.collection_protos[kind.index()]),
                )?);
                let input = arg(0);
                if matches!(input, JsValue::Null | JsValue::Undefined) {
                    return Ok(value);
                }
                let adder = self.get(
                    value.clone(),
                    if kind == CollectionKind::Set {
                        "add"
                    } else {
                        "set"
                    },
                )?;
                if !self.callable(&adder) {
                    return Err(JsError::new(ErrorKind::TypeError, "collection adder"));
                }
                let mut iterator = self.get_iterator(input)?;
                while let Some(entry) = self.iterator_step(&mut iterator)? {
                    let step = (|| {
                        let values = if kind == CollectionKind::Set {
                            vec![entry]
                        } else {
                            if entry.object().is_none() {
                                return Err(JsError::new(
                                    ErrorKind::TypeError,
                                    "map entry must be an object",
                                ));
                            }
                            vec![self.get(entry.clone(), "0")?, self.get(entry, "1")?]
                        };
                        self.call_inner(adder.clone(), value.clone(), values, false, None)?;
                        Ok(())
                    })();
                    if let Err(error) = step {
                        let close = self.close_iterator(&mut iterator);
                        if error.is_resource_limit() {
                            return Err(error);
                        }
                        let _ = close;
                        return Err(error);
                    }
                }
                return Ok(value);
            }
            let id = this
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::TypeError, "collection receiver"))?;
            let ObjectKind::Collection(data) = &self.heap.get(id)?.kind else {
                return Err(JsError::new(ErrorKind::TypeError, "collection receiver"));
            };
            if data.kind != kind {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "collection receiver brand",
                ));
            }
            let method = name.rsplit('.').next().unwrap_or("");
            match method {
                "size" => Ok(JsValue::Number(data.entries.len() as f64)),
                "get" => Ok(data
                    .lookup
                    .get(&key(&arg(0)))
                    .and_then(|index| data.entries.get(index))
                    .map_or(JsValue::Undefined, |(_, v)| v.clone())),
                "has" => Ok(JsValue::Bool(data.lookup.contains_key(&key(&arg(0))))),
                "set" | "add" => {
                    let k = arg(0);
                    let v = if kind == CollectionKind::Set {
                        k.clone()
                    } else {
                        arg(1)
                    };
                    self.collection_insert(id, k, v)?;
                    Ok(this)
                }
                "delete" => {
                    let ObjectKind::Collection(data) = &mut self.heap.get_mut(id)?.kind else {
                        return Err(JsError::new(ErrorKind::TypeError, "collection receiver"));
                    };
                    let present = data
                        .lookup
                        .remove(&key(&arg(0)))
                        .is_some_and(|index| data.entries.remove(&index).is_some());
                    Ok(JsValue::Bool(present))
                }
                "clear" => {
                    let ObjectKind::Collection(data) = &mut self.heap.get_mut(id)?.kind else {
                        return Err(JsError::new(ErrorKind::TypeError, "collection receiver"));
                    };
                    data.entries.clear();
                    data.lookup.clear();
                    Ok(JsValue::Undefined)
                }
                "entries" | "keys" | "values" => {
                    let mode = match method {
                        "keys" => super::iterators::Iteration::Keys,
                        "entries" => super::iterators::Iteration::Entries,
                        _ => super::iterators::Iteration::Values,
                    };
                    Ok(JsValue::Object(self.allocate(
                        ObjectKind::Iterator(Box::new(
                            super::iterators::IteratorData::Collection {
                                source: this,
                                after: None,
                                mode,
                                done: false,
                            },
                        )),
                        Some(self.iterator_proto),
                    )?))
                }
                "forEach" => {
                    let callback = arg(0);
                    if !self.callable(&callback) {
                        return Err(JsError::new(ErrorKind::TypeError, "collection callback"));
                    }
                    let mut after = None;
                    while let Some((index, k, v)) = self.collection_next(id, after)? {
                        self.tick()?;
                        after = Some(index);
                        self.call_inner(
                            callback.clone(),
                            arg(1),
                            vec![v, k, this.clone()],
                            false,
                            None,
                        )?;
                    }
                    Ok(JsValue::Undefined)
                }
                _ => Err(JsError::new(ErrorKind::TypeError, "collection method")),
            }
        })())
    }
}
