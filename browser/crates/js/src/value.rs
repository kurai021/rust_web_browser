//! Traced arena values. Object identity is an index plus generation, not Rc.
use crate::ast::Function;
use crate::{ErrorKind, JsError};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    pub index: usize,
    pub generation: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Arc<str>),
    Object(ObjectId),
    Symbol(u64, Arc<str>),
}
impl JsValue {
    pub fn string(s: impl AsRef<str>) -> Self {
        Self::String(Arc::from(s.as_ref()))
    }
    pub fn truthy(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            _ => true,
        }
    }
    pub fn object(&self) -> Option<ObjectId> {
        if let Self::Object(id) = self {
            Some(*id)
        } else {
            None
        }
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        match self {
            Self::String(s) | Self::Symbol(_, s) => s.len().saturating_add(32),
            _ => 0,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropertyKey {
    String(String),
    Symbol(u64),
}
impl From<&str> for PropertyKey {
    fn from(s: &str) -> Self {
        Self::String(s.into())
    }
}
impl From<String> for PropertyKey {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}
#[derive(Debug, Clone)]
pub enum PropertyValue {
    Data(JsValue),
    Accessor { get: JsValue, set: JsValue },
}
#[derive(Debug, Clone)]
pub struct Property {
    pub value: PropertyValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}
impl Property {
    pub fn data(value: JsValue) -> Self {
        Self {
            value: PropertyValue::Data(value),
            writable: true,
            enumerable: true,
            configurable: true,
        }
    }
    pub(crate) fn retained_bytes(&self, key: &PropertyKey) -> usize {
        let key_bytes = match key {
            PropertyKey::String(s) => s.len(),
            _ => 0,
        };
        192usize
            .saturating_add(key_bytes)
            .saturating_add(match &self.value {
                PropertyValue::Data(v) => v.retained_bytes(),
                PropertyValue::Accessor { get, set } => {
                    get.retained_bytes().saturating_add(set.retained_bytes())
                }
            })
    }
}
#[derive(Debug, Clone)]
pub(crate) struct UserFunction {
    pub function: Arc<Function>,
    pub environment: usize,
    pub lexical_this: Option<JsValue>,
    pub strict: bool,
    pub constructable: bool,
    pub class: bool,
    pub super_constructor: Option<ObjectId>,
    pub super_base: Option<ObjectId>,
}
#[derive(Debug, Clone)]
pub(crate) enum ObjectKind {
    Ordinary,
    Array,
    Function(UserFunction),
    Native(String),
    Bound {
        function: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    },
    Host {
        id: u64,
        class: String,
    },
    Boxed(JsValue),
    RegExp {
        regex: regex::Regex,
        source: String,
        flags: String,
    },
    Date(f64),
    Error,
}
#[derive(Debug, Clone)]
pub(crate) struct Object {
    pub prototype: Option<ObjectId>,
    pub properties: BTreeMap<PropertyKey, Property>,
    pub kind: ObjectKind,
    pub extensible: bool,
}
impl Object {
    pub fn new(kind: ObjectKind, prototype: Option<ObjectId>) -> Self {
        Self {
            prototype,
            kind,
            properties: BTreeMap::new(),
            extensible: true,
        }
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        let kind = match &self.kind {
            ObjectKind::Function(f) => f.function.retained_bytes,
            ObjectKind::Native(s) | ObjectKind::Host { class: s, .. } => s.len(),
            ObjectKind::Bound { args, this, .. } => args.iter().fold(
                this.retained_bytes()
                    .saturating_add(args.len().saturating_mul(32)),
                |n, v| n.saturating_add(v.retained_bytes()),
            ),
            ObjectKind::Boxed(v) => v.retained_bytes(),
            // Each compiled regex is bounded by the builder. Charge its full
            // compilation/cache caps rather than guessing allocator overhead.
            ObjectKind::RegExp { source, flags, .. } => (12 * 1024 * 1024usize)
                .saturating_add(source.len())
                .saturating_add(flags.len()),
            _ => 0,
        };
        self.properties
            .iter()
            .fold(256usize.saturating_add(kind), |n, (k, p)| {
                n.saturating_add(p.retained_bytes(k))
            })
    }
}
pub(crate) struct Slot {
    pub object: Option<Object>,
    pub generation: u64,
}
pub(crate) struct Heap {
    pub slots: Vec<Slot>,
    pub free: Vec<usize>,
}
impl Heap {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
    pub fn alloc(&mut self, object: Object) -> ObjectId {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index];
            slot.generation = slot.generation.wrapping_add(1);
            slot.object = Some(object);
            ObjectId {
                index,
                generation: slot.generation,
            }
        } else {
            let id = ObjectId {
                index: self.slots.len(),
                generation: 0,
            };
            self.slots.push(Slot {
                object: Some(object),
                generation: 0,
            });
            id
        }
    }
    pub fn get(&self, id: ObjectId) -> Result<&Object, JsError> {
        self.slots
            .get(id.index)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.object.as_ref())
            .ok_or_else(|| {
                JsError::new(ErrorKind::TypeError, "object belongs to an expired context")
            })
    }
    pub fn get_mut(&mut self, id: ObjectId) -> Result<&mut Object, JsError> {
        self.slots
            .get_mut(id.index)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.object.as_mut())
            .ok_or_else(|| {
                JsError::new(ErrorKind::TypeError, "object belongs to an expired context")
            })
    }
    pub fn live(&self) -> usize {
        self.slots.len() - self.free.len()
    }
}
