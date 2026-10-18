//! Iterator protocol shared by spread, loops, collections and Promise combinators.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Iteration {
    Keys,
    Values,
    Entries,
}
#[derive(Debug, Clone)]
pub(crate) enum IteratorData {
    Collection {
        source: JsValue,
        after: Option<u64>,
        mode: Iteration,
        done: bool,
    },
    Array {
        source: JsValue,
        index: usize,
        mode: Iteration,
        done: bool,
    },
    String {
        source: Arc<str>,
        index: usize,
    },
}
impl IteratorData {
    pub fn roots(&self) -> Vec<JsValue> {
        match self {
            Self::Array { source, .. } | Self::Collection { source, .. } => vec![source.clone()],
            _ => Vec::new(),
        }
    }
    pub fn retained_bytes(&self) -> usize {
        64 + match self {
            Self::String { source, .. } => source.len(),
            _ => 0,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct IteratorRecord {
    pub object: JsValue,
    pub next: JsValue,
    pub done: bool,
    pub async_from_sync: bool,
}
impl Vm {
    pub(super) fn install_iterators(&mut self) -> Result<(), JsError> {
        self.iterator_proto = self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?;
        let identity = self.native("Iterator.identity")?;
        self.define(
            self.iterator_proto,
            PropertyKey::Symbol(crate::symbols::ITERATOR),
            Property {
                enumerable: false,
                ..Property::data(identity)
            },
        )?;
        let next = self.native("Iterator.next")?;
        self.define(
            self.iterator_proto,
            "next".into(),
            Property {
                enumerable: false,
                ..Property::data(next)
            },
        )?;
        for name in ["keys", "values", "entries"] {
            let function = self.native(&format!("Iterator.array.{name}"))?;
            self.define(
                self.array_proto,
                name.into(),
                Property {
                    enumerable: false,
                    ..Property::data(function.clone())
                },
            )?;
            if name == "values" {
                self.define(
                    self.array_proto,
                    PropertyKey::Symbol(crate::symbols::ITERATOR),
                    Property {
                        enumerable: false,
                        ..Property::data(function)
                    },
                )?;
            }
        }
        let strings = self.native("Iterator.string")?;
        self.define(
            self.string_proto,
            PropertyKey::Symbol(crate::symbols::ITERATOR),
            Property {
                enumerable: false,
                ..Property::data(strings)
            },
        )?;
        if let Some(symbol) = self.get_global("Symbol")?.object() {
            self.symbol_descriptions
                .insert(crate::symbols::ITERATOR, Some("Symbol.iterator".into()));
            for (id, name) in [
                (crate::symbols::SPECIES, "species"),
                (crate::symbols::HAS_INSTANCE, "hasInstance"),
                (crate::symbols::TO_PRIMITIVE, "toPrimitive"),
                (crate::symbols::TO_STRING_TAG, "toStringTag"),
                (crate::symbols::IS_CONCAT_SPREADABLE, "isConcatSpreadable"),
                (crate::symbols::UNSCOPABLES, "unscopables"),
                (crate::symbols::MATCH, "match"),
                (crate::symbols::REPLACE, "replace"),
                (crate::symbols::SEARCH, "search"),
                (crate::symbols::SPLIT, "split"),
                (crate::symbols::ASYNC_ITERATOR, "asyncIterator"),
            ] {
                self.symbol_descriptions
                    .insert(id, Some(format!("Symbol.{name}")));
                self.define(
                    symbol,
                    name.into(),
                    Property {
                        writable: false,
                        enumerable: false,
                        configurable: false,
                        ..Property::data(JsValue::Symbol(id, format!("Symbol.{name}").into()))
                    },
                )?;
            }
            self.symbol_next = self.symbol_next.max(13);
        }
        let unscopables = JsValue::Object(self.allocate(ObjectKind::Ordinary, None)?);
        for name in [
            "keys",
            "values",
            "entries",
            "includes",
            "find",
            "findIndex",
            "copyWithin",
            "fill",
            "flat",
            "flatMap",
            "at",
        ] {
            self.set(unscopables.clone(), name, JsValue::Bool(true))?;
        }
        self.define(
            self.array_proto,
            PropertyKey::Symbol(crate::symbols::UNSCOPABLES),
            Property {
                writable: false,
                enumerable: false,
                configurable: true,
                ..Property::data(unscopables)
            },
        )?;
        Ok(())
    }
    pub(crate) fn iterator_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
    ) -> Option<Result<JsValue, JsError>> {
        let _ = args;
        if !name.starts_with("Iterator.") {
            return None;
        }
        Some((|| match name {
            "Iterator.identity" => Ok(this),
            "Iterator.string" => {
                let source = self.to_string(this)?;
                Ok(JsValue::Object(self.allocate(
                    ObjectKind::Iterator(Box::new(IteratorData::String {
                        source: source.into(),
                        index: 0,
                    })),
                    Some(self.iterator_proto),
                )?))
            }
            name if name.starts_with("Iterator.array.") => {
                if matches!(this, JsValue::Undefined | JsValue::Null) {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "array iterator receiver",
                    ));
                }
                let mode = match name {
                    "Iterator.array.keys" => Iteration::Keys,
                    "Iterator.array.entries" => Iteration::Entries,
                    _ => Iteration::Values,
                };
                Ok(JsValue::Object(self.allocate(
                    ObjectKind::Iterator(Box::new(IteratorData::Array {
                        source: this,
                        index: 0,
                        mode,
                        done: false,
                    })),
                    Some(self.iterator_proto),
                )?))
            }
            "Iterator.next" => {
                let id = this
                    .object()
                    .ok_or_else(|| JsError::new(ErrorKind::TypeError, "iterator receiver"))?;
                let kind = self.heap.get(id)?.kind.clone();
                let ObjectKind::Iterator(data) = kind else {
                    return Err(JsError::new(ErrorKind::TypeError, "iterator receiver"));
                };
                let mut data = *data;
                let value = match &mut data {
                    IteratorData::Collection {
                        source,
                        after,
                        mode,
                        done,
                    } => {
                        if *done {
                            None
                        } else {
                            let id = source.object().ok_or_else(|| {
                                JsError::new(ErrorKind::TypeError, "collection iterator")
                            })?;
                            if let Some((index, key, value)) = self.collection_next(id, *after)? {
                                *after = Some(index);
                                Some(match mode {
                                    Iteration::Keys => key,
                                    Iteration::Values => value,
                                    Iteration::Entries => self.array(vec![key, value])?,
                                })
                            } else {
                                *done = true;
                                None
                            }
                        }
                    }
                    IteratorData::Array {
                        source,
                        index,
                        mode,
                        done,
                    } => {
                        if *done {
                            None
                        } else {
                            let length = self.array_length(source.clone())?;
                            if *index >= length {
                                *done = true;
                                None
                            } else {
                                let n = *index;
                                *index += 1;
                                Some(match mode {
                                    Iteration::Keys => JsValue::Number(n as f64),
                                    Iteration::Values => {
                                        self.get(source.clone(), &n.to_string())?
                                    }
                                    Iteration::Entries => {
                                        let v = self.get(source.clone(), &n.to_string())?;
                                        self.array(vec![JsValue::Number(n as f64), v])?
                                    }
                                })
                            }
                        }
                    }
                    IteratorData::String { source, index } => {
                        if let Some(c) = source.get(*index..).and_then(|s| s.chars().next()) {
                            *index += c.len_utf8();
                            Some(JsValue::string(c.to_string()))
                        } else {
                            None
                        }
                    }
                };
                self.heap.get_mut(id)?.kind = ObjectKind::Iterator(Box::new(data));
                let done = value.is_none();
                self.iterator_result(value.unwrap_or(JsValue::Undefined), done)
            }
            _ => Err(JsError::new(ErrorKind::TypeError, "iterator method")),
        })())
    }
    pub(crate) fn iterator_result(
        &mut self,
        value: JsValue,
        done: bool,
    ) -> Result<JsValue, JsError> {
        let result = self.object()?;
        self.set(result.clone(), "value", value)?;
        self.set(result.clone(), "done", JsValue::Bool(done))?;
        Ok(result)
    }
    pub(crate) fn get_iterator(&mut self, value: JsValue) -> Result<IteratorRecord, JsError> {
        let method = self.get_key(value.clone(), PropertyKey::Symbol(crate::symbols::ITERATOR))?;
        if !self.callable(&method) {
            return Err(JsError::new(ErrorKind::TypeError, "value is not iterable"));
        }
        let object = self.call_inner(method, value, Vec::new(), false, None)?;
        if object.object().is_none() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "iterator is not an object",
            ));
        }
        let next = self.get(object.clone(), "next")?;
        if !self.callable(&next) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "iterator.next is not callable",
            ));
        }
        Ok(IteratorRecord {
            object,
            next,
            done: false,
            async_from_sync: false,
        })
    }
    pub(crate) fn get_async_iterator(&mut self, value: JsValue) -> Result<IteratorRecord, JsError> {
        let method = self.get_key(
            value.clone(),
            PropertyKey::Symbol(crate::symbols::ASYNC_ITERATOR),
        )?;
        if matches!(method, JsValue::Undefined | JsValue::Null) {
            let mut record = self.get_iterator(value)?;
            record.async_from_sync = true;
            return Ok(record);
        }
        if !self.callable(&method) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "Symbol.asyncIterator is not callable",
            ));
        }
        let object = self.call_inner(method, value, Vec::new(), false, None)?;
        if object.object().is_none() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "async iterator is not an object",
            ));
        }
        let next = self.get(object.clone(), "next")?;
        if !self.callable(&next) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "async iterator.next is not callable",
            ));
        }
        Ok(IteratorRecord {
            object,
            next,
            done: false,
            async_from_sync: false,
        })
    }
    pub(crate) fn async_iterator_result(
        &mut self,
        iterator: &IteratorRecord,
        result: JsValue,
    ) -> Result<JsValue, JsError> {
        if !iterator.async_from_sync {
            return Ok(result);
        }
        if result.object().is_none() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "iterator result is not an object",
            ));
        }
        let done = self.get(result.clone(), "done")?.truthy();
        let value = self.get(result, "value")?;
        self.unwrap_async_iterator_value(value, done)
    }
    pub(crate) fn close_async_iterator(
        &mut self,
        iterator: &mut IteratorRecord,
    ) -> Result<Option<JsValue>, JsError> {
        if iterator.done {
            return Ok(None);
        }
        iterator.done = true;
        let method = self.get(iterator.object.clone(), "return")?;
        if matches!(method, JsValue::Undefined | JsValue::Null) {
            return Ok(None);
        }
        let result = self.call_inner(method, iterator.object.clone(), Vec::new(), false, None)?;
        self.async_iterator_result(iterator, result).map(Some)
    }
    pub(crate) fn iterator_step(
        &mut self,
        iterator: &mut IteratorRecord,
    ) -> Result<Option<JsValue>, JsError> {
        if iterator.done {
            return Ok(None);
        }
        self.tick()?;
        let result = self.call_inner(
            iterator.next.clone(),
            iterator.object.clone(),
            Vec::new(),
            false,
            None,
        )?;
        if result.object().is_none() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "iterator result is not an object",
            ));
        }
        if self.get(result.clone(), "done")?.truthy() {
            iterator.done = true;
            Ok(None)
        } else {
            Ok(Some(self.get(result, "value")?))
        }
    }
    pub(crate) fn close_iterator(&mut self, iterator: &mut IteratorRecord) -> Result<(), JsError> {
        if iterator.done {
            return Ok(());
        }
        iterator.done = true;
        let method = self.get(iterator.object.clone(), "return")?;
        if matches!(method, JsValue::Undefined | JsValue::Null) {
            return Ok(());
        }
        if !self.callable(&method) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "iterator.return is not callable",
            ));
        }
        let result = self.call_inner(method, iterator.object.clone(), Vec::new(), false, None)?;
        if result.object().is_none() {
            return Err(JsError::new(ErrorKind::TypeError, "iterator.return result"));
        }
        Ok(())
    }
    pub fn iterable_values(&mut self, value: JsValue) -> Result<Vec<JsValue>, JsError> {
        let mut iterator = self.get_iterator(value)?;
        let mut out = Vec::new();
        loop {
            match self.iterator_step(&mut iterator) {
                Ok(Some(value)) => {
                    self.check_temporary(out.len().saturating_add(1) * 64)?;
                    out.push(value);
                    if out.len() > 100_000 {
                        let _ = self.close_iterator(&mut iterator);
                        return Err(JsError::new(ErrorKind::MemoryLimit, "iterator item quota"));
                    }
                }
                Ok(None) => return Ok(out),
                Err(error) => return Err(error),
            }
        }
    }
}
