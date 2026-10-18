//! Bounded lexical interpreter and tracing GC. No platform I/O is exposed.
use crate::ast::*;
use crate::value::*;
use crate::{parse, ErrorKind, JsError, ParseOpts, Span};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
#[path = "bigint_builtins.rs"]
mod bigint_builtins;
#[path = "collections.rs"]
pub(crate) mod collections;
#[path = "descriptors.rs"]
pub(crate) mod descriptors;
#[path = "execution.rs"]
pub(crate) mod execution;
#[path = "iterators.rs"]
pub(crate) mod iterators;
#[path = "modules.rs"]
pub(crate) mod modules;
#[path = "promises.rs"]
pub(crate) mod promises;
#[path = "proxy.rs"]
pub(crate) mod proxy;
#[path = "symbol_builtins.rs"]
mod symbol_builtins;

pub struct HostResult {
    pub value: JsValue,
    pub calls: Vec<(JsValue, JsValue, Vec<JsValue>)>,
    pub finish: Option<u64>,
    pub catch_callback_errors: bool,
}
impl HostResult {
    pub fn value(value: JsValue) -> Self {
        Self {
            value,
            calls: Vec::new(),
            finish: None,
            catch_callback_errors: false,
        }
    }
}
pub trait HostHooks {
    /// Independent trampolines may share a context, but never an active borrow.
    fn fork(&self) -> Option<Box<dyn HostHooks>> {
        None
    }
    fn before_callback(
        &mut self,
        _vm: &mut Vm,
        _function: &JsValue,
        _this: &JsValue,
        _args: &[JsValue],
    ) -> bool {
        true
    }
    fn print(&mut self, _s: &str) {}
    fn get(&mut self, _vm: &mut Vm, _id: u64, _key: &str) -> Result<Option<JsValue>, JsError> {
        Ok(None)
    }
    fn set(
        &mut self,
        _vm: &mut Vm,
        _id: u64,
        _key: &str,
        _value: JsValue,
    ) -> Result<bool, JsError> {
        Ok(false)
    }
    fn call(
        &mut self,
        _vm: &mut Vm,
        _name: &str,
        _this: JsValue,
        _args: &[JsValue],
    ) -> Result<HostResult, JsError> {
        Err(JsError::new(ErrorKind::TypeError, "unknown host method"))
    }
    fn finish(&mut self, _vm: &mut Vm, _token: u64) -> Result<JsValue, JsError> {
        Ok(JsValue::Undefined)
    }
    /// HTML activation stays native; Web APIs decide navigation/download policy.
    fn activate_link(
        &mut self,
        _vm: &mut Vm,
        _url: &str,
        _download: Option<&str>,
        _target: &str,
    ) -> Result<bool, JsError> {
        Ok(false)
    }
    /// Connected DOM listeners are roots. Detached cycles are traced only from
    /// the host objects that the JS mark phase actually reaches.
    fn roots(&self, _live_hosts: &HashSet<u64>) -> Vec<JsValue> {
        Vec::new()
    }
    fn host_references(&self, _live_hosts: &HashSet<u64>) -> Vec<u64> {
        Vec::new()
    }
    /// Trace native graph edges to already-created wrappers (e.g. DOM children).
    /// Candidate identities are realm-local and never exposed to JavaScript.
    fn trace_native_edges(
        &self,
        _live_hosts: &HashSet<u64>,
        _candidates: &HashSet<u64>,
    ) -> Vec<u64> {
        Vec::new()
    }
    fn sweep(&mut self, _live_hosts: &HashSet<u64>) {}
    fn allow_dynamic_code(&self) -> bool {
        true
    }
    fn retained_bytes(&self) -> usize {
        0
    }
    fn run_event_loop(&mut self, _vm: &mut Vm) {}
    fn import_module(
        &mut self,
        _vm: &mut Vm,
        _url: &str,
        _referrer: &str,
    ) -> Result<JsValue, JsError> {
        Err(JsError::new(
            ErrorKind::TypeError,
            "no module loader in this realm",
        ))
    }
    fn after_operation(&mut self, _vm: &mut Vm) -> Result<(), JsError> {
        Ok(())
    }
    fn microtask(&mut self, _vm: &mut Vm, _token: u64) -> Result<HostResult, JsError> {
        Ok(HostResult::value(JsValue::Undefined))
    }
}
pub struct NullHost;
impl HostHooks for NullHost {}
#[derive(Clone)]
pub struct Limits {
    /// Native-stack headroom. Embedders with an 8 MiB JS thread may raise this
    /// to 4 MiB; the default leaves room in Rust's 2 MiB test/worker threads.
    pub max_native_stack_bytes: usize,
    pub navigation: Option<(Arc<AtomicU64>, u64)>,
    pub max_steps: u64,
    pub max_duration: Duration,
    pub max_call_depth: usize,
    pub max_objects: usize,
    pub max_environments: usize,
    pub max_string_bytes: usize,
    pub max_memory_bytes: usize,
    pub cancel: Arc<AtomicBool>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_native_stack_bytes: 1024 * 1024,
            navigation: None,
            max_steps: 2_000_000,
            max_duration: Duration::from_secs(5),
            max_call_depth: 128,
            max_objects: 500_000,
            max_environments: 500_000,
            max_string_bytes: 8 * 1024 * 1024,
            max_memory_bytes: 512 * 1024 * 1024,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
#[derive(Clone)]
struct Binding {
    value: JsValue,
    initialized: bool,
    mutable: bool,
    kind: DeclKind,
}
#[derive(Clone)]
struct Environment {
    aliases: BTreeMap<String, (usize, String)>,
    parent: Option<usize>,
    bindings: BTreeMap<String, Binding>,
    function: bool,
    this: JsValue,
}
#[derive(Clone)]
enum Completion {
    Normal(JsValue),
    Return(JsValue),
    Break(Option<String>),
    Continue(Option<String>),
}
enum Reference {
    Name(String),
    Property(JsValue, PropertyKey),
}
pub struct Vm {
    stack_base: usize,
    call_stack: Vec<(String, String, Span)>,
    gc_requested: bool,
    current_module: Option<usize>,
    symbol_proto: ObjectId,
    symbol_registry: BTreeMap<String, JsValue>,
    registered_symbols: HashSet<u64>,
    symbol_descriptions: HashMap<u64, Option<String>>,
    source_url: String,
    modules: Vec<modules::Module>,
    module_urls: HashMap<String, usize>,
    module_loads: HashMap<String, JsValue>,
    proxy_depth: usize,
    collection_protos: [ObjectId; 3],
    generator_proto: ObjectId,
    async_generator_proto: ObjectId,
    ast_reference: bool,
    bigint_proto: ObjectId,
    promise_proto: ObjectId,
    promise_ctor: ObjectId,
    aggregate_error_proto: ObjectId,
    iterator_proto: ObjectId,
    microtasks: std::collections::VecDeque<promises::Job>,
    rejected_promises: Vec<ObjectId>,
    promise_notifications: Vec<promises::PromiseNotification>,
    async_errors: Vec<JsError>,
    draining_microtasks: bool,
    pub(crate) heap: Heap,
    environments: Vec<Option<Environment>>,
    free_env: Vec<usize>,
    pub(crate) global: ObjectId,
    pub(crate) object_proto: ObjectId,
    pub(crate) function_proto: ObjectId,
    pub(crate) array_proto: ObjectId,
    pub(crate) string_proto: ObjectId,
    pub(crate) number_proto: ObjectId,
    pub(crate) boolean_proto: ObjectId,
    pub(crate) date_proto: ObjectId,
    pub(crate) regexp_proto: ObjectId,
    pub(crate) error_proto: ObjectId,
    host: Option<Box<dyn HostHooks>>,
    host_template: Option<Box<dyn HostHooks>>,
    dynamic_code_allowed: bool,
    pub limits: Limits,
    steps: u64,
    started: Instant,
    depth: usize,
    active: Vec<usize>,
    strict: bool,
    pub(crate) symbol_next: u64,
    host_objects: HashMap<u64, ObjectId>,
    last: JsValue,
    pins: Vec<JsValue>,
    pub console: Vec<String>,
    memory_bytes: usize,
}
impl Vm {
    pub fn new(host: impl HostHooks + 'static) -> Self {
        let host_template = host.fork();
        let mut heap = Heap::new();
        let object_proto = heap.alloc(Object::new(ObjectKind::Ordinary, None));
        let function_proto = heap.alloc(Object::new(ObjectKind::Ordinary, Some(object_proto)));
        let global = heap.alloc(Object::new(ObjectKind::Ordinary, Some(object_proto)));
        let mut vm = Self {
            stack_base: 0,
            call_stack: Vec::new(),
            gc_requested: false,
            current_module: None,
            symbol_proto: object_proto,
            symbol_registry: BTreeMap::new(),
            registered_symbols: HashSet::new(),
            symbol_descriptions: HashMap::new(),
            source_url: String::new(),
            modules: Vec::new(),
            module_urls: HashMap::new(),
            module_loads: HashMap::new(),
            proxy_depth: 0,
            collection_protos: [object_proto; 3],
            generator_proto: object_proto,
            async_generator_proto: object_proto,
            ast_reference: false,
            bigint_proto: object_proto,
            promise_proto: object_proto,
            promise_ctor: object_proto,
            aggregate_error_proto: object_proto,
            iterator_proto: object_proto,
            microtasks: Default::default(),
            rejected_promises: Vec::new(),
            promise_notifications: Vec::new(),
            async_errors: Vec::new(),
            draining_microtasks: false,
            heap,
            environments: vec![Some(Environment {
                aliases: BTreeMap::new(),
                parent: None,
                bindings: BTreeMap::new(),
                function: true,
                this: JsValue::Object(global),
            })],
            free_env: Vec::new(),
            global,
            object_proto,
            function_proto,
            array_proto: object_proto,
            string_proto: object_proto,
            number_proto: object_proto,
            boolean_proto: object_proto,
            date_proto: object_proto,
            regexp_proto: object_proto,
            error_proto: object_proto,
            host: Some(Box::new(host)),
            host_template,
            dynamic_code_allowed: true,
            limits: Limits::default(),
            steps: 0,
            started: Instant::now(),
            depth: 0,
            active: Vec::new(),
            strict: false,
            symbol_next: 1,
            host_objects: HashMap::new(),
            last: JsValue::Undefined,
            pins: Vec::new(),
            console: Vec::new(),
            memory_bytes: 1024,
        };
        crate::builtins::install(&mut vm);
        let _ = vm.install_iterators();
        let _ = vm.install_promises();
        let _ = vm.install_bigint();
        let _ = vm.install_generators();
        let _ = vm.install_collections();
        let _ = vm.install_proxy();
        let _ = vm.install_symbols();
        vm
    }
    pub fn global_value(&self) -> JsValue {
        JsValue::Object(self.global)
    }
    pub fn parse_json(&mut self, source: &str) -> Result<JsValue, JsError> {
        crate::builtins::call(
            self,
            "JSON.parse",
            JsValue::Undefined,
            vec![JsValue::string(source)],
            false,
            None,
        )
    }
    pub fn stringify_json(&mut self, value: JsValue) -> Result<String, JsError> {
        let value = crate::builtins::call(
            self,
            "JSON.stringify",
            JsValue::Undefined,
            vec![value],
            false,
            None,
        )?;
        self.to_string(value)
    }
    pub fn set_global_host(&mut self, id: u64, class: &str) -> Result<(), JsError> {
        self.heap.get_mut(self.global)?.kind = ObjectKind::Host {
            id,
            class: class.into(),
        };
        Ok(())
    }
    pub fn set_global(&mut self, name: &str, value: JsValue) -> Result<(), JsError> {
        self.define(
            self.global,
            PropertyKey::from(name),
            Property::data(value.clone()),
        )?;
        self.reserve_binding(0, name, &value)?;
        if let Some(env) = self.environments[0].as_mut() {
            env.bindings.insert(
                name.into(),
                Binding {
                    value,
                    initialized: true,
                    mutable: true,
                    kind: DeclKind::Var,
                },
            );
        }
        Ok(())
    }
    pub fn get_global(&mut self, name: &str) -> Result<JsValue, JsError> {
        self.lookup(0, name)
    }
    pub fn replace_host(&mut self, host: impl HostHooks + 'static) {
        self.dynamic_code_allowed = host.allow_dynamic_code();
        self.host_template = host.fork();
        self.host = Some(Box::new(host));
    }
    pub(crate) fn emit_console(&mut self, line: &str) -> Result<(), JsError> {
        self.with_host(|host, _| {
            host.print(line);
            Ok(())
        })
    }
    fn with_host<T>(
        &mut self,
        f: impl FnOnce(&mut dyn HostHooks, &mut Self) -> Result<T, JsError>,
    ) -> Result<T, JsError> {
        if let Some(host) = &self.host {
            self.dynamic_code_allowed = host.allow_dynamic_code();
        }
        let primary = self.host.is_some();
        let mut host = self
            .host
            .take()
            .or_else(|| self.host_template.as_ref().and_then(|h| h.fork()))
            .ok_or_else(|| JsError::new(ErrorKind::Error, "reentrant host operation"))?;
        let result = f(&mut *host, self);
        if primary {
            self.host = Some(host);
        }
        result
    }
    pub fn host_object(&mut self, id: u64, class: &str) -> Result<JsValue, JsError> {
        if let Some(object) = self.host_objects.get(&id).copied() {
            if self.heap.get(object).is_ok() {
                return Ok(JsValue::Object(object));
            }
        }
        let object = self.allocate(
            ObjectKind::Host {
                id,
                class: class.into(),
            },
            Some(self.object_proto),
        )?;
        self.host_objects.insert(id, object);
        Ok(JsValue::Object(object))
    }
    pub fn host_id(&self, value: &JsValue) -> Option<u64> {
        match value
            .object()
            .and_then(|id| self.heap.get(id).ok())
            .map(|o| &o.kind)
        {
            Some(ObjectKind::Host { id, .. }) => Some(*id),
            _ => None,
        }
    }
    pub fn set_host_prototype(
        &mut self,
        value: &JsValue,
        prototype: JsValue,
    ) -> Result<(), JsError> {
        if let (Some(id), Some(prototype)) = (value.object(), prototype.object()) {
            if matches!(
                self.heap.get(id)?.kind,
                ObjectKind::Host { .. } | ObjectKind::Ordinary
            ) {
                self.heap.get_mut(id)?.prototype = Some(prototype);
            }
        }
        Ok(())
    }
    pub fn is_array(&self, value: &JsValue) -> bool {
        value
            .object()
            .and_then(|id| self.heap.get(id).ok())
            .is_some_and(|o| matches!(o.kind, ObjectKind::Array))
    }
    pub(crate) fn box_for_clone(&mut self, value: JsValue) -> Result<JsValue, JsError> {
        let id = self.allocate(ObjectKind::Boxed(value.clone()), Some(self.object_proto))?;
        let object = JsValue::Object(id);
        self.fill_box_for_clone(&object, value)?;
        Ok(object)
    }
    pub(crate) fn fill_box_for_clone(
        &mut self,
        object: &JsValue,
        value: JsValue,
    ) -> Result<(), JsError> {
        let prototype = match value {
            JsValue::BigInt(_) => self.bigint_proto,
            JsValue::String(_) => self.string_proto,
            JsValue::Number(_) => self.number_proto,
            JsValue::Bool(_) => self.boolean_proto,
            _ => self.object_proto,
        };
        let id = object
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "clone boxed primitive"))?;
        self.heap.get_mut(id)?.kind = ObjectKind::Boxed(value);
        self.heap.get_mut(id)?.prototype = Some(prototype);
        Ok(())
    }
    pub(crate) fn regexp_for_clone(
        &mut self,
        source: &str,
        flags: &str,
    ) -> Result<JsValue, JsError> {
        self.regexp(source, flags)
    }
    pub fn date_value(&self, value: &JsValue) -> Option<f64> {
        match value
            .object()
            .and_then(|id| self.heap.get(id).ok())
            .map(|o| &o.kind)
        {
            Some(ObjectKind::Date(value)) => Some(*value),
            _ => None,
        }
    }
    pub fn native(&mut self, name: &str) -> Result<JsValue, JsError> {
        let id = self.allocate(ObjectKind::Native(name.into()), Some(self.function_proto))?;
        self.define(
            id,
            "name".into(),
            Property {
                enumerable: false,
                ..Property::data(JsValue::string(name))
            },
        )?;
        Ok(JsValue::Object(id))
    }
    pub fn object(&mut self) -> Result<JsValue, JsError> {
        Ok(JsValue::Object(
            self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?,
        ))
    }
    pub fn array(&mut self, values: Vec<JsValue>) -> Result<JsValue, JsError> {
        let id = self.allocate(ObjectKind::Array, Some(self.array_proto))?;
        let length = values.len();
        for (i, value) in values.into_iter().enumerate() {
            self.define(id, i.to_string().into(), Property::data(value))?;
        }
        self.define(
            id,
            "length".into(),
            Property {
                enumerable: false,
                configurable: false,
                ..Property::data(JsValue::Number(length as f64))
            },
        )?;
        Ok(JsValue::Object(id))
    }
    pub(crate) fn allocate(
        &mut self,
        kind: ObjectKind,
        prototype: Option<ObjectId>,
    ) -> Result<ObjectId, JsError> {
        if self.heap.live() >= self.limits.max_objects {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "JS heap limit exceeded",
            ));
        }
        let object = Object::new(kind, prototype);
        self.reserve_memory(object.retained_bytes())?;
        Ok(self.heap.alloc(object))
    }
    pub fn accounted_bytes(&self) -> usize {
        self.memory_bytes
            .saturating_add(self.host.as_ref().map_or(0, |h| h.retained_bytes()))
    }
    /// Native host loops must not wait for an AST tick to notice cancellation.
    pub fn checkpoint(&self) -> Result<(), JsError> {
        if self.limits.cancel.load(Ordering::Relaxed)
            || self
                .limits
                .navigation
                .as_ref()
                .is_some_and(|(token, id)| token.load(Ordering::Relaxed) != *id)
        {
            return Err(JsError::new(ErrorKind::Cancelled, "context replaced"));
        }
        if self.started.elapsed() > self.limits.max_duration {
            return Err(JsError::new(
                ErrorKind::Timeout,
                "script exceeded time limit",
            ));
        }
        Ok(())
    }
    pub fn check_host_bytes(&self, bytes: usize) -> Result<(), JsError> {
        self.check_temporary(bytes)
    }
    fn reserve_memory(&mut self, bytes: usize) -> Result<(), JsError> {
        let total = self.memory_bytes.saturating_add(bytes);
        if total > self.limits.max_memory_bytes {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "page memory budget exceeded",
            ));
        }
        self.memory_bytes = total;
        Ok(())
    }
    pub(crate) fn check_temporary(&self, bytes: usize) -> Result<(), JsError> {
        if self.memory_bytes.saturating_add(bytes) > self.limits.max_memory_bytes {
            Err(JsError::new(
                ErrorKind::MemoryLimit,
                "temporary allocation budget exceeded",
            ))
        } else {
            Ok(())
        }
    }
    pub fn check_string_bytes(&self, bytes: usize) -> Result<(), JsError> {
        if bytes > self.limits.max_string_bytes {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "string limit exceeded",
            ));
        }
        self.check_temporary(bytes.saturating_mul(2))
    }
    fn check_value(&self, value: &JsValue) -> Result<(), JsError> {
        if let JsValue::String(s) | JsValue::Symbol(_, s) = value {
            self.check_string_bytes(s.len())?;
        }
        Ok(())
    }
    fn reserve_binding(&mut self, env: usize, name: &str, value: &JsValue) -> Result<(), JsError> {
        self.check_value(value)?;
        let old = self
            .scope(env)?
            .bindings
            .get(name)
            .map_or(0, |b| 192 + name.len() + b.value.retained_bytes());
        let new = 192usize
            .saturating_add(name.len())
            .saturating_add(value.retained_bytes());
        if new >= old {
            self.reserve_memory(new - old)?;
        } else {
            self.memory_bytes = self.memory_bytes.saturating_sub(old - new);
        }
        Ok(())
    }
    fn store_property(
        &mut self,
        id: ObjectId,
        key: PropertyKey,
        property: Property,
    ) -> Result<(), JsError> {
        if let PropertyKey::String(s) = &key {
            self.check_string_bytes(s.len())?;
        }
        match &property.value {
            PropertyValue::Data(v) => self.check_value(v)?,
            PropertyValue::Accessor { get, set } => {
                self.check_value(get)?;
                self.check_value(set)?;
            }
        }
        let old = self
            .heap
            .get(id)?
            .properties
            .get(&key)
            .map_or(0, |p| p.retained_bytes(&key));
        let new = property.retained_bytes(&key);
        if new >= old {
            self.reserve_memory(new - old)?;
        } else {
            self.memory_bytes = self.memory_bytes.saturating_sub(old - new);
        }
        if !self.heap.get(id)?.properties.contains_key(&key) {
            self.reserve_memory(
                48 + match &key {
                    PropertyKey::String(s) => s.len(),
                    _ => 0,
                },
            )?;
            self.heap.get_mut(id)?.property_order.push(key.clone());
        }
        self.heap.get_mut(id)?.properties.insert(key, property);
        Ok(())
    }
    pub fn define(
        &mut self,
        id: ObjectId,
        key: PropertyKey,
        property: Property,
    ) -> Result<(), JsError> {
        if matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            if self.reflect_define(JsValue::Object(id), key, property)? {
                return Ok(());
            }
            return Err(JsError::new(
                ErrorKind::TypeError,
                "Proxy defineProperty returned false",
            ));
        }
        let object = self.heap.get_mut(id)?;
        if !object.extensible && !object.properties.contains_key(&key) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "object is not extensible",
            ));
        }
        if let Some(old) = object.properties.get(&key) {
            if !old.configurable
                && (old.enumerable != property.enumerable
                    || property.configurable
                    || (!old.writable && property.writable))
            {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "non-configurable property",
                ));
            }
        }
        self.store_property(id, key, property)
    }
    pub fn get(&mut self, value: JsValue, key: &str) -> Result<JsValue, JsError> {
        self.get_key(value, PropertyKey::from(key))
    }
    pub fn get_key(&mut self, value: JsValue, key: PropertyKey) -> Result<JsValue, JsError> {
        self.get_key_receiver(value.clone(), key, value)
    }
    fn get_key_receiver(
        &mut self,
        value: JsValue,
        key: PropertyKey,
        receiver: JsValue,
    ) -> Result<JsValue, JsError> {
        if matches!(value, JsValue::Null | JsValue::Undefined) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "property access on null/undefined",
            ));
        }
        if let (JsValue::String(s), PropertyKey::String(k)) = (&value, &key) {
            if k == "length" {
                return Ok(JsValue::Number(s.encode_utf16().count() as f64));
            }
            if let Ok(i) = k.parse::<usize>() {
                if let Some(unit) = s.encode_utf16().nth(i) {
                    return Ok(JsValue::string(String::from_utf16_lossy(&[unit])));
                }
            }
        }
        let mut current = match &value {
            JsValue::Object(id) => Some(*id),
            JsValue::String(_) => Some(self.string_proto),
            JsValue::Number(_) => Some(self.number_proto),
            JsValue::BigInt(_) => Some(self.bigint_proto),
            JsValue::Symbol(..) => Some(self.symbol_proto),
            JsValue::Bool(_) => Some(self.boolean_proto),
            _ => Some(self.object_proto),
        };
        let mut seen = HashSet::new();
        while let Some(id) = current {
            let object = self.heap.get(id)?;
            let (module, proxy, host) = match &object.kind {
                ObjectKind::ModuleNamespace(module) => (Some(*module), false, None),
                ObjectKind::Proxy(_) => (None, true, None),
                ObjectKind::Host { id, .. } => (None, false, Some(*id)),
                _ => (None, false, None),
            };
            let property = object.properties.get(&key).cloned();
            let prototype = object.prototype;
            if let (Some(module), PropertyKey::String(_)) = (module, &key) {
                return self.module_get(module, &key);
            }
            if proxy {
                return self.proxy_get(id, key, receiver);
            }
            if let (Some(host), PropertyKey::String(k)) = (host, &key) {
                if let Some(v) = self.with_host(|h, vm| h.get(vm, host, k))? {
                    return Ok(v);
                }
            }
            if let Some(p) = property {
                return match &p.value {
                    PropertyValue::Data(v) => Ok(v.clone()),
                    PropertyValue::Accessor { get, .. } => {
                        if matches!(get, JsValue::Undefined) {
                            Ok(JsValue::Undefined)
                        } else {
                            self.call_inner(get.clone(), receiver.clone(), Vec::new(), false, None)
                        }
                    }
                };
            }
            if !seen.insert(id) {
                break;
            }
            current = prototype;
        }
        Ok(JsValue::Undefined)
    }
    pub fn set(&mut self, value: JsValue, key: &str, new: JsValue) -> Result<(), JsError> {
        self.set_key(value, PropertyKey::from(key), new)
    }
    pub fn set_key(
        &mut self,
        value: JsValue,
        key: PropertyKey,
        new: JsValue,
    ) -> Result<(), JsError> {
        let Some(id) = value.object() else {
            if self.strict {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "assignment to primitive",
                ));
            }
            return Ok(());
        };
        if matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
            let success = self.proxy_set(id, key, new, value)?;
            if !success && self.strict {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "Proxy set returned false",
                ));
            }
            return Ok(());
        }
        if matches!(self.heap.get(id)?.kind, ObjectKind::ModuleNamespace(_)) {
            return if self.strict {
                Err(JsError::new(
                    ErrorKind::TypeError,
                    "module namespace is read only",
                ))
            } else {
                Ok(())
            };
        }
        if let (ObjectKind::Host { id: host, .. }, PropertyKey::String(k)) =
            (self.heap.get(id)?.kind.clone(), &key)
        {
            let large = matches!(&new, JsValue::String(s) if s.len() > 4096);
            if self.with_host(|h, vm| h.set(vm, host, k, new.clone()))? {
                self.with_host(|h, vm| h.after_operation(vm))?;
                self.checkpoint()?;
                if large {
                    self.finish_memory_check(JsValue::Undefined)?;
                }
                return Ok(());
            }
        }
        let mut current = Some(id);
        let mut seen = HashSet::new();
        while let Some(parent) = current {
            if !seen.insert(parent) {
                break;
            }
            let object = self.heap.get(parent)?;
            let property = object.properties.get(&key).cloned();
            let prototype = object.prototype;
            if let Some(p) = property {
                match &p.value {
                    PropertyValue::Accessor { set, .. } => {
                        if matches!(set, JsValue::Undefined) {
                            if self.strict {
                                return Err(JsError::new(
                                    ErrorKind::TypeError,
                                    "getter-only property",
                                ));
                            }
                            return Ok(());
                        }
                        self.call_inner(set.clone(), value, vec![new], false, None)?;
                        return Ok(());
                    }
                    PropertyValue::Data(_) if !p.writable => {
                        if self.strict {
                            return Err(JsError::new(ErrorKind::TypeError, "read-only property"));
                        }
                        return Ok(());
                    }
                    _ => break,
                }
            }
            current = prototype;
        }
        if matches!(self.heap.get(id)?.kind, ObjectKind::Array) {
            if let PropertyKey::String(k) = &key {
                if k == "length" {
                    let n = self.to_number(new.clone())?;
                    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
                        return Err(JsError::new(ErrorKind::RangeError, "invalid array length"));
                    }
                    self.heap.get_mut(id)?.properties.retain(|k,_|!matches!(k,PropertyKey::String(s)if s.parse::<u32>().is_ok_and(|i|i>=n as u32)));
                    let object = self.heap.get_mut(id)?;
                    object
                        .property_order
                        .retain(|key| object.properties.contains_key(key));
                } else if let Ok(index) = k.parse::<u32>() {
                    if index != u32::MAX {
                        let len = self.array_length(JsValue::Object(id))?;
                        if index as usize >= len {
                            self.store_property(
                                id,
                                "length".into(),
                                Property {
                                    enumerable: false,
                                    configurable: false,
                                    ..Property::data(JsValue::Number(index as f64 + 1.0))
                                },
                            )?;
                        }
                    }
                }
            }
        }
        let object = self.heap.get_mut(id)?;
        if !object.extensible && !object.properties.contains_key(&key) {
            if self.strict {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "object is not extensible",
                ));
            }
            return Ok(());
        }
        let property = if let Some(p) = object.properties.get(&key) {
            Property {
                value: PropertyValue::Data(new),
                ..p.clone()
            }
        } else {
            Property::data(new)
        };
        self.store_property(id, key, property)
    }
    pub fn delete(&mut self, value: JsValue, key: PropertyKey) -> Result<bool, JsError> {
        if let Some(id) = value.object() {
            if let ObjectKind::ModuleNamespace(module) = self.heap.get(id)?.kind {
                if matches!(&key,PropertyKey::String(name)if self.module_names(module)?.contains(name))
                {
                    if self.strict {
                        return Err(JsError::new(
                            ErrorKind::TypeError,
                            "module export cannot be deleted",
                        ));
                    }
                    return Ok(false);
                }
            }
        }
        if let Some(id) = value.object() {
            if matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
                return self.proxy_delete(id, key);
            }
        }
        let Some(id) = value.object() else {
            return Ok(true);
        };
        if self
            .heap
            .get(id)?
            .properties
            .get(&key)
            .is_some_and(|p| !p.configurable)
        {
            if self.strict {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "delete non-configurable property",
                ));
            }
            return Ok(false);
        }
        if let Some(property) = self.heap.get_mut(id)?.properties.remove(&key) {
            self.heap.get_mut(id)?.property_order.retain(|k| *k != key);
            self.memory_bytes = self
                .memory_bytes
                .saturating_sub(property.retained_bytes(&key));
        }
        Ok(true)
    }
    pub fn keys(&mut self, value: &JsValue, enumerable: bool) -> Result<Vec<PropertyKey>, JsError> {
        if let Some(id) = value.object() {
            if let ObjectKind::ModuleNamespace(module) = self.heap.get(id)?.kind {
                let mut keys = self
                    .module_names(module)?
                    .into_iter()
                    .map(PropertyKey::String)
                    .collect::<Vec<_>>();
                if !enumerable {
                    keys.extend(self.heap.get(id)?.properties.keys().cloned());
                }
                return Ok(keys);
            }
        }
        if let Some(id) = value.object() {
            if matches!(self.heap.get(id)?.kind, ObjectKind::Proxy(_)) {
                return self.proxy_keys(id, enumerable);
            }
        }
        let Some(id) = value.object() else {
            let mut keys = Vec::new();
            if let JsValue::String(value) = value {
                keys.extend(
                    (0..value.encode_utf16().count())
                        .map(|index| PropertyKey::String(index.to_string())),
                );
                if !enumerable {
                    keys.push("length".into());
                }
            }
            return Ok(keys);
        };
        let object = self.heap.get(id)?;
        let mut keys = object
            .property_order
            .iter()
            .filter(|key| {
                object
                    .properties
                    .get(*key)
                    .is_some_and(|p| !enumerable || p.enumerable)
            })
            .cloned()
            .collect::<Vec<_>>();
        fn index(key: &PropertyKey) -> Option<u32> {
            let PropertyKey::String(key) = key else {
                return None;
            };
            let index = key.parse::<u32>().ok()?;
            (index != u32::MAX && index.to_string() == *key).then_some(index)
        }
        keys.sort_by(|a, b| match (index(a), index(b)) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => matches!(a, PropertyKey::Symbol(_)).cmp(&matches!(b, PropertyKey::Symbol(_))),
        });
        Ok(keys)
    }
    pub fn array_length(&mut self, value: JsValue) -> Result<usize, JsError> {
        let n = self.get(value, "length")?;
        let n = self.to_number(n)?;
        Ok(if n.is_nan() || n <= 0.0 {
            0
        } else {
            n.min(1_000_000.0) as usize
        })
    }
    pub fn array_values(&mut self, value: JsValue) -> Result<Vec<JsValue>, JsError> {
        if let JsValue::String(s) = &value {
            return Ok(s.chars().map(|c| JsValue::string(c.to_string())).collect());
        }
        let len = self.array_length(value.clone())?;
        self.check_temporary(len.saturating_mul(32))?;
        let mut values = Vec::new();
        for i in 0..len {
            self.tick()?;
            values.push(self.get(value.clone(), &i.to_string())?);
        }
        Ok(values)
    }
    pub fn to_string(&mut self, value: JsValue) -> Result<String, JsError> {
        match value {
            JsValue::BigInt(value) => value.to_radix(10),
            JsValue::Undefined => Ok("undefined".into()),
            JsValue::Null => Ok("null".into()),
            JsValue::Bool(b) => Ok(b.to_string()),
            JsValue::Number(n) => Ok(number_string(n)),
            JsValue::String(s) => Ok(s.to_string()),
            JsValue::Symbol(..) => Err(JsError::new(
                ErrorKind::TypeError,
                "cannot coerce Symbol to string",
            )),
            object => {
                let primitive = self.coerce_primitive(object, false)?;
                self.to_string(primitive)
            }
        }
    }
    pub fn to_number(&mut self, value: JsValue) -> Result<f64, JsError> {
        match value {
            JsValue::BigInt(_) => Err(JsError::new(
                ErrorKind::TypeError,
                "cannot implicitly convert BigInt to Number",
            )),
            JsValue::Undefined => Ok(f64::NAN),
            JsValue::Null => Ok(0.0),
            JsValue::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
            JsValue::Number(n) => Ok(n),
            JsValue::String(s) => {
                let s = s.trim();
                if s.is_empty() {
                    return Ok(0.0);
                }
                if s == "Infinity" || s == "+Infinity" {
                    return Ok(f64::INFINITY);
                }
                if s == "-Infinity" {
                    return Ok(f64::NEG_INFINITY);
                }
                if s.starts_with("0x") || s.starts_with("0X") {
                    return Ok(u64::from_str_radix(&s[2..], 16).map_or(f64::NAN, |n| n as f64));
                }
                Ok(s.parse().unwrap_or(f64::NAN))
            }
            JsValue::Symbol(..) => Err(JsError::new(
                ErrorKind::TypeError,
                "cannot coerce Symbol to number",
            )),
            object => {
                let p = self.coerce_primitive(object, true)?;
                self.to_number(p)
            }
        }
    }
    pub(crate) fn coerce_primitive(
        &mut self,
        value: JsValue,
        number: bool,
    ) -> Result<JsValue, JsError> {
        self.coerce_primitive_hint(value, if number { "number" } else { "string" })
    }
    fn coerce_primitive_hint(&mut self, value: JsValue, hint: &str) -> Result<JsValue, JsError> {
        let Some(id) = value.object() else {
            return Ok(value);
        };
        let exotic = self.get_key(
            value.clone(),
            PropertyKey::Symbol(crate::symbols::TO_PRIMITIVE),
        )?;
        if !matches!(exotic, JsValue::Undefined | JsValue::Null) {
            if !self.callable(&exotic) {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "Symbol.toPrimitive is not callable",
                ));
            }
            let result = self.call_inner(
                exotic,
                value.clone(),
                vec![JsValue::string(hint)],
                false,
                None,
            )?;
            if result.object().is_some() {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "Symbol.toPrimitive returned an object",
                ));
            }
            return Ok(result);
        }
        let number = hint == "number"
            || hint == "default" && !matches!(self.heap.get(id)?.kind, ObjectKind::Date(_));
        for key in if number {
            ["valueOf", "toString"]
        } else {
            ["toString", "valueOf"]
        } {
            let method = self.get(value.clone(), key)?;
            if self.callable(&method) {
                let result = self.call_inner(method, value.clone(), Vec::new(), false, None)?;
                if !matches!(result, JsValue::Object(_)) {
                    return Ok(result);
                }
            }
        }
        Err(JsError::new(
            ErrorKind::TypeError,
            "cannot convert object to primitive",
        ))
    }
    pub fn property_key(&mut self, value: JsValue) -> Result<PropertyKey, JsError> {
        let value = if value.object().is_some() {
            self.coerce_primitive(value, false)?
        } else {
            value
        };
        if let JsValue::Symbol(id, _) = value {
            Ok(PropertyKey::Symbol(id))
        } else {
            Ok(self.to_string(value)?.into())
        }
    }
    pub fn callable(&self, value: &JsValue) -> bool {
        if let Some(id) = value.object() {
            if let Ok(Object {
                kind: ObjectKind::Proxy(data),
                ..
            }) = self.heap.get(id)
            {
                return data.callable;
            }
        }
        value
            .object()
            .and_then(|id| self.heap.get(id).ok())
            .is_some_and(|o| {
                matches!(
                    o.kind,
                    ObjectKind::Function(_)
                        | ObjectKind::Native(_)
                        | ObjectKind::Bound { .. }
                        | ObjectKind::PromiseClosure(_)
                        | ObjectKind::ProxyRevoker(_)
                )
            })
    }
    fn env(&mut self, parent: usize, function: bool, this: JsValue) -> Result<usize, JsError> {
        if self.environments.len() - self.free_env.len() >= self.limits.max_environments {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "environment limit exceeded",
            ));
        }
        self.reserve_memory(256usize.saturating_add(this.retained_bytes()))?;
        let environment = Environment {
            aliases: BTreeMap::new(),
            parent: Some(parent),
            bindings: BTreeMap::new(),
            function,
            this,
        };
        if let Some(id) = self.free_env.pop() {
            self.environments[id] = Some(environment);
            Ok(id)
        } else {
            let id = self.environments.len();
            self.environments.push(Some(environment));
            Ok(id)
        }
    }
    fn scope(&self, id: usize) -> Result<&Environment, JsError> {
        self.environments
            .get(id)
            .and_then(Option::as_ref)
            .ok_or_else(|| JsError::new(ErrorKind::ReferenceError, "expired environment"))
    }
    fn lookup(&mut self, mut env: usize, name: &str) -> Result<JsValue, JsError> {
        loop {
            let scope = self.scope(env)?;
            if let Some((target, name)) = scope.aliases.get(name).cloned() {
                return self.lookup(target, &name);
            }
            if let Some(b) = scope.bindings.get(name) {
                if !b.initialized {
                    return Err(JsError::new(
                        ErrorKind::ReferenceError,
                        "binding accessed before initialization",
                    ));
                }
                if env == 0 && b.kind == DeclKind::Var {
                    return self.get(JsValue::Object(self.global), name);
                }
                return Ok(b.value.clone());
            }
            if let Some(parent) = scope.parent {
                env = parent;
            } else {
                break;
            }
        }
        let key: PropertyKey = name.into();
        if self.has(JsValue::Object(self.global), key)? {
            self.get(JsValue::Object(self.global), name)
        } else {
            Err(JsError::new(
                ErrorKind::ReferenceError,
                format!("{name} is not defined"),
            ))
        }
    }
    fn binding_exists(&mut self, mut env: usize, name: &str) -> Result<bool, JsError> {
        loop {
            let scope = self.scope(env)?;
            if scope.bindings.contains_key(name) || scope.aliases.contains_key(name) {
                return Ok(true);
            }
            let Some(parent) = scope.parent else {
                break;
            };
            env = parent;
        }
        self.has(JsValue::Object(self.global), name.into())
    }
    fn assign_name(&mut self, mut env: usize, name: &str, value: JsValue) -> Result<(), JsError> {
        loop {
            let scope = self.scope(env)?;
            if scope.aliases.contains_key(name) {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "assignment to import binding",
                ));
            }
            if let Some(b) = scope.bindings.get(name) {
                if !b.initialized {
                    return Err(JsError::new(
                        ErrorKind::ReferenceError,
                        "uninitialized binding",
                    ));
                }
                if !b.mutable {
                    return Err(JsError::new(ErrorKind::TypeError, "assignment to const"));
                }
                let global_property = env == 0 && b.kind == DeclKind::Var;
                self.reserve_binding(env, name, &value)?;
                if let Some(scope) = self.environments[env].as_mut() {
                    if let Some(b) = scope.bindings.get_mut(name) {
                        b.value = value.clone();
                    }
                }
                if global_property {
                    self.set(JsValue::Object(self.global), name, value)?;
                }
                return Ok(());
            }
            if let Some(parent) = scope.parent {
                env = parent;
            } else {
                break;
            }
        }
        if self.strict {
            return Err(JsError::new(
                ErrorKind::ReferenceError,
                format!("{name} is not defined"),
            ));
        }
        self.set_global(name, value)
    }
    fn declare_name(
        &mut self,
        mut env: usize,
        name: &str,
        kind: DeclKind,
        value: JsValue,
        initialize: bool,
    ) -> Result<(), JsError> {
        if kind == DeclKind::Var {
            while !self.scope(env)?.function {
                env = self.scope(env)?.parent.unwrap_or(0);
            }
        }
        let existing = self.scope(env)?.bindings.get(name).cloned();
        if existing.is_none() || initialize {
            self.reserve_binding(env, name, &value)?;
        }
        if let Some(existing) = existing {
            if existing.initialized && kind != DeclKind::Var {
                return Err(JsError::new(
                    ErrorKind::SyntaxError,
                    "duplicate lexical binding",
                ));
            }
            if initialize {
                if let Some(scope) = self.environments[env].as_mut() {
                    scope.bindings.insert(
                        name.into(),
                        Binding {
                            value: value.clone(),
                            initialized: true,
                            mutable: kind != DeclKind::Const,
                            kind,
                        },
                    );
                }
            }
        } else if let Some(scope) = self.environments[env].as_mut() {
            scope.bindings.insert(
                name.into(),
                Binding {
                    value: value.clone(),
                    initialized: initialize || kind == DeclKind::Var,
                    mutable: kind != DeclKind::Const,
                    kind,
                },
            );
        }
        if env == 0 && kind == DeclKind::Var && initialize {
            self.define(self.global, name.into(), Property::data(value))?;
        }
        Ok(())
    }
    fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
        match p {
            Pattern::Name(s) => out.push(s.clone()),
            Pattern::Array(v, r) => {
                for p in v.iter().flatten() {
                    Self::pattern_names(p, out);
                }
                if let Some(p) = r {
                    Self::pattern_names(p, out);
                }
            }
            Pattern::Object(v, r) => {
                for (_, p) in v {
                    Self::pattern_names(p, out);
                }
                if let Some(p) = r {
                    Self::pattern_names(p, out);
                }
            }
            Pattern::Default(p, _) | Pattern::Rest(p) => Self::pattern_names(p, out),
        }
    }
    fn bind(
        &mut self,
        p: &Pattern,
        value: JsValue,
        env: usize,
        kind: DeclKind,
    ) -> Result<(), JsError> {
        match p {
            Pattern::Name(s) => self.declare_name(env, s, kind, value, true),
            Pattern::Default(p, default) => {
                let value = if value == JsValue::Undefined {
                    self.expr(default, env)?
                } else {
                    value
                };
                self.bind(p, value, env, kind)
            }
            Pattern::Rest(p) => self.bind(p, value, env, kind),
            Pattern::Array(parts, rest) => {
                let values = self.array_values(value)?;
                for (i, p) in parts.iter().enumerate() {
                    if let Some(p) = p {
                        self.bind(
                            p,
                            values.get(i).cloned().unwrap_or(JsValue::Undefined),
                            env,
                            kind,
                        )?;
                    }
                }
                if let Some(rest) = rest {
                    let array = self.array(values.into_iter().skip(parts.len()).collect())?;
                    self.bind(rest, array, env, kind)?;
                }
                Ok(())
            }
            Pattern::Object(parts, rest) => {
                if matches!(value, JsValue::Null | JsValue::Undefined) {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "destructure null/undefined",
                    ));
                }
                let mut used = HashSet::new();
                for (name, p) in parts {
                    let key = self.prop_name(name, env)?;
                    used.insert(key.clone());
                    let v = self.get_key(value.clone(), key)?;
                    self.bind(p, v, env, kind)?;
                }
                if let Some(rest) = rest {
                    let object = self.object()?;
                    for key in self.keys(&value, true)? {
                        if !used.contains(&key) {
                            let v = self.get_key(value.clone(), key.clone())?;
                            self.set_key(object.clone(), key, v)?;
                        }
                    }
                    self.bind(rest, object, env, kind)?;
                }
                Ok(())
            }
        }
    }
    fn hoist(&mut self, body: &[Stmt], env: usize, lexical: bool) -> Result<(), JsError> {
        for stmt in body {
            match &stmt.kind {
                StmtKind::Declare(kind, ds) => {
                    if *kind == DeclKind::Var || lexical {
                        for d in ds {
                            let mut names = Vec::new();
                            Self::pattern_names(&d.pattern, &mut names);
                            for name in names {
                                if *kind == DeclKind::Var
                                    && self.scope(env)?.bindings.contains_key(&name)
                                {
                                    continue;
                                }
                                self.declare_name(env, &name, *kind, JsValue::Undefined, false)?;
                            }
                        }
                    }
                }
                StmtKind::Function(name, f) => {
                    let value = self.function(f.clone(), env)?;
                    self.declare_name(env, name, DeclKind::Var, value, true)?;
                }
                StmtKind::Class(name, _) if lexical => {
                    self.declare_name(env, name, DeclKind::Let, JsValue::Undefined, false)?
                }
                StmtKind::Block(body) => self.hoist(body, env, false)?,
                StmtKind::If(_, yes, no) => {
                    self.hoist(std::slice::from_ref(yes), env, false)?;
                    if let Some(no) = no {
                        self.hoist(std::slice::from_ref(no), env, false)?;
                    }
                }
                StmtKind::While(_, s) | StmtKind::DoWhile(s, _) | StmtKind::Label(_, s) => {
                    self.hoist(std::slice::from_ref(s), env, false)?
                }
                StmtKind::For(init, _, _, body) => {
                    if let Some(init) = init {
                        self.hoist(std::slice::from_ref(init), env, false)?;
                    }
                    self.hoist(std::slice::from_ref(body), env, false)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub(crate) fn tick(&mut self) -> Result<(), JsError> {
        let stack_marker = 0u8;
        let stack = (&stack_marker as *const u8) as usize;
        if self.stack_base != 0
            && self.stack_base.abs_diff(stack) > self.limits.max_native_stack_bytes
        {
            return Err(JsError::new(
                ErrorKind::RangeError,
                "native JavaScript stack budget exceeded",
            ));
        }
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return Err(JsError::new(
                ErrorKind::Timeout,
                "script instruction budget exceeded",
            ));
        }
        if self.steps.is_multiple_of(128) {
            if self
                .limits
                .navigation
                .as_ref()
                .is_some_and(|(token, id)| token.load(Ordering::Relaxed) != *id)
            {
                return Err(JsError::new(ErrorKind::Cancelled, "context replaced"));
            }
            if self.limits.cancel.load(Ordering::Relaxed) {
                return Err(JsError::new(ErrorKind::Cancelled, "script cancelled"));
            }
            if self.started.elapsed() > self.limits.max_duration {
                return Err(JsError::new(
                    ErrorKind::Timeout,
                    "script exceeded time limit",
                ));
            }
        }
        if self.steps.is_multiple_of(4096) && self.accounted_bytes() > self.limits.max_memory_bytes
        {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "page memory budget exceeded",
            ));
        }
        Ok(())
    }
    /// Start a bounded native task before decoding service results or messages.
    pub fn begin_task(&mut self) {
        let stack_marker = 0u8;
        self.stack_base = (&stack_marker as *const u8) as usize;
        self.steps = 0;
        self.started = Instant::now();
        self.depth = 0;
    }
    /// Native modal waiting is not script execution; instruction/call caps remain.
    pub fn exclude_host_wait(&mut self, wait: Duration) {
        if let Some(started) = self.started.checked_add(wait) {
            self.started = started;
        }
    }
    pub fn eval(&mut self, program: &Program) -> Result<JsValue, JsError> {
        if program.module {
            return Err(JsError::new(
                ErrorKind::SyntaxError,
                "modules require a linked module record",
            ));
        }
        self.begin_task();
        let old = self.strict;
        self.strict = program.strict;
        let result = self
            .eval_body(program, 0)
            .and_then(|value| self.finish_memory_check(value));
        self.strict = old;
        if let Ok(value) = &result {
            self.last = value.clone();
        }
        result
    }
    /// Execute immutable realm-independent bytecode without reparsing/recompiling.
    /// Module chunks must use the linker instead of the classic global scope.
    pub fn eval_compiled(&mut self, code: Arc<crate::bytecode::Chunk>) -> Result<JsValue, JsError> {
        if code.module || code.function {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "expected classic script bytecode",
            ));
        }
        self.begin_task();
        let strict = self.strict;
        self.strict = code.strict;
        let result = self
            .execute_chunk(code, 0)
            .and_then(|value| self.finish_memory_check(value));
        self.strict = strict;
        if let Ok(value) = &result {
            self.last = value.clone();
        }
        result
    }
    pub(crate) fn eval_body(&mut self, program: &Program, env: usize) -> Result<JsValue, JsError> {
        if !self.ast_reference {
            return self.execute_chunk(crate::bytecode::compile(program)?, env);
        }
        self.hoist(&program.body, env, true)?;
        match self.statements(&program.body, env)? {
            Completion::Normal(v) | Completion::Return(v) => Ok(v),
            _ => Err(JsError::new(
                ErrorKind::SyntaxError,
                "unresolved loop control",
            )),
        }
    }
    pub fn eval_source(&mut self, source: &str) -> Result<JsValue, JsError> {
        let program = parse(source, ParseOpts::default()).map_err(|e| JsError {
            kind: ErrorKind::SyntaxError,
            message: e.message,
            line: e.line,
            column: e.column,
            stack: Vec::new(),
            thrown: None,
        })?;
        self.eval(&program)
    }
    /// Owned Level 1 oracle for bytecode migration tests and benches.
    pub fn eval_ast_reference(&mut self, program: &Program) -> Result<JsValue, JsError> {
        let old = self.ast_reference;
        self.ast_reference = true;
        let result = self.eval(program);
        self.ast_reference = old;
        result
    }
    /// Compile an already CSP-approved inline handler inside the current task.
    /// Resetting task limits here would let nested dispatch bypass the killer.
    pub fn compile_event_handler(&mut self, source: &str) -> Result<JsValue, JsError> {
        let source = format!("(function(event){{{source}}});");
        let program = parse(&source, ParseOpts::default())
            .map_err(|e| JsError::new(ErrorKind::SyntaxError, e.to_string()))?;
        let strict = self.strict;
        self.strict = false;
        let result = self.eval_body(&program, 0);
        self.strict = strict;
        result
    }
    pub fn call(
        &mut self,
        function: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    ) -> Result<JsValue, JsError> {
        self.begin_task();
        let result = self
            .call_inner(function, this, args, false, None)
            .and_then(|value| self.finish_memory_check(value));
        if let Ok(v) = &result {
            self.last = v.clone();
        }
        result
    }
    pub fn call_in_task(
        &mut self,
        function: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    ) -> Result<JsValue, JsError> {
        self.call_inner(function, this, args, false, None)
            .and_then(|v| self.finish_memory_check(v))
    }
    pub fn run_event_loop(&mut self) {
        if let Err(error) = self.drain_microtasks() {
            self.async_errors.push(error);
        }
        let _ = self.with_host(|host, vm| {
            host.run_event_loop(vm);
            Ok(())
        });
    }
    fn finish_memory_check(&self, value: JsValue) -> Result<JsValue, JsError> {
        if self.accounted_bytes() > self.limits.max_memory_bytes {
            Err(JsError::new(
                ErrorKind::MemoryLimit,
                "page memory budget exceeded",
            ))
        } else {
            Ok(value)
        }
    }
    fn statements(&mut self, body: &[Stmt], env: usize) -> Result<Completion, JsError> {
        let mut last = JsValue::Undefined;
        for stmt in body {
            match self.stmt(stmt, env)? {
                Completion::Normal(v) => last = v,
                other => return Ok(other),
            }
        }
        Ok(Completion::Normal(last))
    }
    fn stmt(&mut self, stmt: &Stmt, env: usize) -> Result<Completion, JsError> {
        self.tick()?;
        let result = self.stmt_inner(stmt, env);
        result.map_err(|mut e| {
            if e.line == 0 {
                e.line = stmt.span.line;
                e.column = stmt.span.column;
            }
            e
        })
    }
    fn stmt_inner(&mut self, stmt: &Stmt, env: usize) -> Result<Completion, JsError> {
        self.stmt_labeled(stmt, env, None)
    }
    fn stmt_labeled(
        &mut self,
        stmt: &Stmt,
        env: usize,
        label: Option<&str>,
    ) -> Result<Completion, JsError> {
        let undefined = Completion::Normal(JsValue::Undefined);
        match &stmt.kind {
            StmtKind::Empty | StmtKind::Function(..) => Ok(undefined),
            StmtKind::Expr(e) => Ok(Completion::Normal(self.expr(e, env)?)),
            StmtKind::Block(body) => {
                let child = self.env(env, false, self.scope(env)?.this.clone())?;
                self.hoist(body, child, true)?;
                self.statements(body, child)
            }
            StmtKind::Declare(kind, ds) => {
                for d in ds {
                    if let Some(init) = &d.init {
                        let value = self.expr(init, env)?;
                        self.bind(&d.pattern, value, env, *kind)?;
                    } else if *kind != DeclKind::Var {
                        self.bind(&d.pattern, JsValue::Undefined, env, *kind)?;
                    }
                }
                Ok(undefined)
            }
            StmtKind::Class(name, class) => {
                let value = self.class(class, env)?;
                self.declare_name(env, name, DeclKind::Let, value, true)?;
                Ok(undefined)
            }
            StmtKind::If(test, yes, no) => {
                if self.expr(test, env)?.truthy() {
                    self.stmt(yes, env)
                } else if let Some(no) = no {
                    self.stmt(no, env)
                } else {
                    Ok(undefined)
                }
            }
            StmtKind::While(test, body) => {
                while self.expr(test, env)?.truthy() {
                    match self.stmt(body, env)? {
                        Completion::Continue(Some(name)) if label == Some(name.as_str()) => {}
                        Completion::Break(Some(name)) if label == Some(name.as_str()) => break,
                        Completion::Normal(_) | Completion::Continue(None) => {}
                        Completion::Break(None) => break,
                        other => return Ok(other),
                    }
                }
                Ok(undefined)
            }
            StmtKind::DoWhile(body, test) => {
                loop {
                    match self.stmt(body, env)? {
                        Completion::Continue(Some(name)) if label == Some(name.as_str()) => {}
                        Completion::Break(Some(name)) if label == Some(name.as_str()) => break,
                        Completion::Normal(_) | Completion::Continue(None) => {}
                        Completion::Break(None) => break,
                        other => return Ok(other),
                    }
                    if !self.expr(test, env)?.truthy() {
                        break;
                    }
                }
                Ok(undefined)
            }
            StmtKind::For(init, test, update, body) => {
                let loop_env = self.env(env, false, self.scope(env)?.this.clone())?;
                if let Some(init) = init {
                    self.hoist(std::slice::from_ref(init), loop_env, true)?;
                    self.stmt(init, loop_env)?;
                }
                let mut iter_env = loop_env;
                loop {
                    self.tick()?;
                    if let Some(test) = test {
                        if !self.expr(test, iter_env)?.truthy() {
                            break;
                        }
                    }
                    match self.stmt(body, iter_env)? {
                        Completion::Continue(Some(name)) if label == Some(name.as_str()) => {}
                        Completion::Break(Some(name)) if label == Some(name.as_str()) => break,
                        Completion::Normal(_) | Completion::Continue(None) => {}
                        Completion::Break(None) => break,
                        other => return Ok(other),
                    }
                    let next = self.env(env, false, self.scope(iter_env)?.this.clone())?;
                    let bindings = self.scope(iter_env)?.bindings.clone();
                    self.reserve_memory(bindings.iter().fold(0usize, |n, (k, b)| {
                        n.saturating_add(192 + k.len() + b.value.retained_bytes())
                    }))?;
                    if let Some(scope) = self.environments[next].as_mut() {
                        scope.bindings = bindings;
                    }
                    iter_env = next;
                    if let Some(update) = update {
                        self.expr(update, iter_env)?;
                    }
                }
                Ok(undefined)
            }
            StmtKind::ForEach(binding, source, body, of, asynchronous) => {
                if *asynchronous {
                    return Err(JsError::new(
                        ErrorKind::SyntaxError,
                        "async iteration requires resumable bytecode",
                    ));
                }
                let source = self.expr(source, env)?;
                let values = if *of {
                    self.array_values(source)?
                } else {
                    self.enumerable_keys(source)?
                        .into_iter()
                        .filter_map(|k| {
                            if let PropertyKey::String(s) = k {
                                Some(JsValue::string(s))
                            } else {
                                None
                            }
                        })
                        .collect()
                };
                for value in values {
                    let child = self.env(env, false, self.scope(env)?.this.clone())?;
                    match binding {
                        ForBinding::Declare(kind, p) => self.bind(p, value, child, *kind)?,
                        ForBinding::Target(target) => self.assign_target(target, value, env)?,
                    }
                    match self.stmt(body, child)? {
                        Completion::Continue(Some(name)) if label == Some(name.as_str()) => {}
                        Completion::Break(Some(name)) if label == Some(name.as_str()) => break,
                        Completion::Normal(_) | Completion::Continue(None) => {}
                        Completion::Break(None) => break,
                        other => return Ok(other),
                    }
                }
                Ok(undefined)
            }
            StmtKind::Return(value) => Ok(Completion::Return(if let Some(v) = value {
                self.expr(v, env)?
            } else {
                JsValue::Undefined
            })),
            StmtKind::Throw(e) => {
                let value = self.expr(e, env)?;
                let message = self
                    .to_string(value.clone())
                    .unwrap_or_else(|_| "uncaught exception".into());
                let mut error = JsError::new(ErrorKind::Error, message);
                error.thrown = Some(value);
                Err(error)
            }
            StmtKind::Break(label) => Ok(Completion::Break(label.clone())),
            StmtKind::Continue(label) => Ok(Completion::Continue(label.clone())),
            StmtKind::Try(body, catch, finally) => {
                let mut result = self.stmt(body, env);
                if result.as_ref().is_err_and(|e| e.is_resource_limit()) {
                    return result;
                }
                if let (Err(error), Some((pat, catch))) = (&result, catch) {
                    if !error.is_resource_limit() {
                        let value = if let Some(value) = &error.thrown {
                            value.clone()
                        } else {
                            self.error_value(error.clone())?
                        };
                        let child = self.env(env, false, self.scope(env)?.this.clone())?;
                        self.bind(pat, value, child, DeclKind::Let)?;
                        result = self.stmt(catch, child);
                    }
                }
                if let Some(finally) = finally {
                    let final_result = self.stmt(finally, env);
                    if !matches!(final_result, Ok(Completion::Normal(_))) {
                        result = final_result;
                    }
                }
                result
            }
            StmtKind::Switch(test, cases) => {
                let value = self.expr(test, env)?;
                let mut start = None;
                let mut default = None;
                for (i, (test, _)) in cases.iter().enumerate() {
                    if let Some(test) = test {
                        if strict_equal(&value, &self.expr(test, env)?) {
                            start = Some(i);
                            break;
                        }
                    } else {
                        default = Some(i);
                    }
                }
                let mut last = JsValue::Undefined;
                if let Some(start) = start.or(default) {
                    for (_, body) in cases.iter().skip(start) {
                        match self.statements(body, env)? {
                            Completion::Normal(v) => last = v,
                            Completion::Break(None) => return Ok(Completion::Normal(last)),
                            other => return Ok(other),
                        }
                    }
                }
                Ok(Completion::Normal(last))
            }
            StmtKind::Label(label, body) => match self.stmt_labeled(body, env, Some(label))? {
                Completion::Break(Some(l)) if l == *label => Ok(undefined),
                other => Ok(other),
            },
        }
    }
    fn function(&mut self, function: Arc<Function>, env: usize) -> Result<JsValue, JsError> {
        let code = crate::bytecode::compile_body(&function.body, true)?;
        let lexical_this = if function.arrow {
            Some(self.scope(env)?.this.clone())
        } else {
            None
        };
        let constructable = !function.arrow && !function.asynchronous && !function.generator;
        let strict = self.strict || function.strict;
        let id = self.allocate(
            ObjectKind::Function(UserFunction {
                module: self.current_module,
                source_url: self.source_url.clone(),
                code,
                function: function.clone(),
                environment: env,
                lexical_this,
                strict,
                constructable,
                class: false,
                super_constructor: None,
                super_base: None,
            }),
            Some(self.function_proto),
        )?;
        if constructable {
            let prototype = self.object()?;
            self.set(prototype.clone(), "constructor", JsValue::Object(id))?;
            self.define(
                id,
                "prototype".into(),
                Property {
                    enumerable: false,
                    ..Property::data(prototype)
                },
            )?;
        }
        self.define(
            id,
            "name".into(),
            Property {
                enumerable: false,
                ..Property::data(JsValue::string(function.name.as_deref().unwrap_or("")))
            },
        )?;
        Ok(JsValue::Object(id))
    }
    fn class(&mut self, class: &Class, env: usize) -> Result<JsValue, JsError> {
        let superclass = if let Some(expr) = &class.extends {
            Some(self.expr(expr, env)?)
        } else {
            None
        };
        let super_id = superclass.as_ref().and_then(JsValue::object);
        if superclass
            .as_ref()
            .is_some_and(|s| *s != JsValue::Null && !self.callable(s))
        {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "extends target is not a constructor",
            ));
        }
        let constructor = class
            .methods
            .iter()
            .find(|m| !m.is_static && matches!(&m.name,PropertyName::String(s)if s=="constructor"));
        let default = Arc::new(Function {
            asynchronous: false,
            generator: false,
            name: class.name.clone(),
            params: Vec::new(),
            body: Vec::new(),
            arrow: false,
            strict: true,
            span: Span::default(),
            retained_bytes: 256,
        });
        let value = self.function(constructor.map_or(default, |m| m.function.clone()), env)?;
        let id = value
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::Error, "class allocation"))?;
        let super_base = if let Some(super_id) = super_id {
            self.get(JsValue::Object(super_id), "prototype")?.object()
        } else {
            None
        };
        if let ObjectKind::Function(f) = &mut self.heap.get_mut(id)?.kind {
            f.class = true;
            f.super_constructor = super_id;
            f.super_base = super_base;
        }
        if let Some(super_id) = super_id {
            self.heap.get_mut(id)?.prototype = Some(super_id);
            let super_proto = self.get(JsValue::Object(super_id), "prototype")?;
            let prototype = self.get(value.clone(), "prototype")?;
            if let Some(proto) = prototype.object() {
                self.heap.get_mut(proto)?.prototype = super_proto.object();
            }
        }
        let prototype = self.get(value.clone(), "prototype")?;
        for method in &class.methods {
            if !method.is_static
                && matches!(&method.name,PropertyName::String(s)if s=="constructor")
            {
                continue;
            }
            let key = self.prop_name(&method.name, env)?;
            let f = self.function(method.function.clone(), env)?;
            if let Some(id) = f.object() {
                if let ObjectKind::Function(function) = &mut self.heap.get_mut(id)?.kind {
                    function.constructable = false;
                    function.super_base = if method.is_static {
                        super_id
                    } else {
                        super_base
                    };
                }
            }
            self.method(
                if method.is_static {
                    value.clone()
                } else {
                    prototype.clone()
                },
                key,
                f,
                method.kind,
                false,
            )?;
        }
        Ok(value)
    }
    fn method(
        &mut self,
        object: JsValue,
        key: PropertyKey,
        function: JsValue,
        kind: MethodKind,
        enumerable: bool,
    ) -> Result<(), JsError> {
        let id = object
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "method target"))?;
        let property = if kind == MethodKind::Method {
            Property {
                enumerable,
                ..Property::data(function)
            }
        } else {
            let existing = self.heap.get(id)?.properties.get(&key).cloned();
            let (mut get, mut set) = match existing.map(|p| p.value) {
                Some(PropertyValue::Accessor { get, set }) => (get, set),
                _ => (JsValue::Undefined, JsValue::Undefined),
            };
            if kind == MethodKind::Get {
                get = function
            } else {
                set = function
            };
            Property {
                value: PropertyValue::Accessor { get, set },
                writable: false,
                enumerable,
                configurable: true,
            }
        };
        self.define(id, key, property)
    }
    fn prop_name(&mut self, name: &PropertyName, env: usize) -> Result<PropertyKey, JsError> {
        match name {
            PropertyName::String(s) => Ok(s.clone().into()),
            PropertyName::Computed(expr) => {
                let value = self.expr(expr, env)?;
                self.property_key(value)
            }
        }
    }
    fn arguments(&mut self, args: &[Argument], env: usize) -> Result<Vec<JsValue>, JsError> {
        let mut values = Vec::new();
        for arg in args {
            let value = self.expr(&arg.expr, env)?;
            if arg.spread {
                values.extend(self.array_values(value)?);
            } else {
                values.push(value);
            }
            if values.len() > 100_000 {
                return Err(JsError::new(ErrorKind::MemoryLimit, "argument limit"));
            }
        }
        Ok(values)
    }
    fn expr(&mut self, expr: &Expr, env: usize) -> Result<JsValue, JsError> {
        if !self.ast_reference {
            return self.execute_chunk(crate::bytecode::compile_expression(expr)?, env);
        }
        self.tick()?;
        let result = self.expr_inner(expr, env);
        let result = result.and_then(|value| {
            self.check_value(&value)?;
            Ok(value)
        });
        result.map_err(|mut e| {
            if e.line == 0 {
                e.line = expr.span.line;
                e.column = expr.span.column;
            }
            e
        })
    }
    fn expr_inner(&mut self, expr: &Expr, env: usize) -> Result<JsValue, JsError> {
        match &expr.kind {
            ExprKind::Import(_) | ExprKind::ImportMeta => Err(JsError::new(
                ErrorKind::SyntaxError,
                "module expressions require bytecode",
            )),
            ExprKind::Await(_) | ExprKind::Yield(..) => Err(JsError::new(
                ErrorKind::SyntaxError,
                "suspension requires a resumable execution frame",
            )),
            ExprKind::Literal(l) => match l {
                Literal::BigInt(value) => Ok(JsValue::BigInt(Arc::new(
                    crate::bigint::BigInt::parse(value)?,
                ))),
                Literal::Undefined => Ok(JsValue::Undefined),
                Literal::Null => Ok(JsValue::Null),
                Literal::Bool(b) => Ok(JsValue::Bool(*b)),
                Literal::Number(n) => Ok(JsValue::Number(*n)),
                Literal::String(s) => Ok(JsValue::string(s)),
                Literal::RegExp(p, f) => self.regexp(p, f),
            },
            ExprKind::Name(name) => self.lookup(env, name),
            ExprKind::This => Ok(self.scope(env)?.this.clone()),
            ExprKind::Function(f) => self.function(f.clone(), env),
            ExprKind::Class(c) => self.class(c, env),
            ExprKind::Array(parts) => {
                let mut values = Vec::new();
                let mut holes = Vec::new();
                for part in parts {
                    if let Some(part) = part {
                        let value = self.expr(&part.expr, env)?;
                        if part.spread {
                            values.extend(self.array_values(value)?);
                        } else {
                            values.push(value);
                        }
                    } else {
                        holes.push(values.len());
                        values.push(JsValue::Undefined);
                    }
                }
                let array = self.array(values)?;
                for i in holes {
                    self.delete(array.clone(), i.to_string().into())?;
                }
                Ok(array)
            }
            ExprKind::Object(props) => {
                let object = self.object()?;
                for prop in props {
                    match prop {
                        ObjectProperty::Value(key, value) => {
                            let key = self.prop_name(key, env)?;
                            let value = self.expr(value, env)?;
                            self.set_key(object.clone(), key, value)?;
                        }
                        ObjectProperty::Method(key, f, kind) => {
                            let key = self.prop_name(key, env)?;
                            let f = self.function(f.clone(), env)?;
                            self.method(object.clone(), key, f, *kind, true)?;
                        }
                        ObjectProperty::Spread(source) => {
                            let value = self.expr(source, env)?;
                            for key in self.keys(&value, true)? {
                                let v = self.get_key(value.clone(), key.clone())?;
                                self.set_key(object.clone(), key, v)?;
                            }
                        }
                    }
                }
                Ok(object)
            }
            ExprKind::Template(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        TemplatePart::Text(s) => out.push_str(s),
                        TemplatePart::Expression(e) => {
                            let v = self.expr(e, env)?;
                            out.push_str(&self.to_string(v)?);
                        }
                    }
                    if out.len() > self.limits.max_string_bytes {
                        return Err(JsError::new(ErrorKind::MemoryLimit, "string limit"));
                    }
                }
                Ok(JsValue::string(out))
            }
            ExprKind::Member(object, key) => {
                let is_super = matches!(&object.kind, ExprKind::Name(s) if s == "super");
                let object = if is_super {
                    self.lookup(env, "\0super-base")?
                } else {
                    self.expr(object, env)?
                };
                let key = self.expr(key, env)?;
                let key = self.property_key(key)?;
                let receiver = if is_super {
                    self.scope(env)?.this.clone()
                } else {
                    object.clone()
                };
                self.get_key_receiver(object, key, receiver)
            }
            ExprKind::Call(callee, args) => {
                let (function, this) = if let ExprKind::Member(object, key) = &callee.kind {
                    let is_super = matches!(&object.kind,ExprKind::Name(s)if s=="super");
                    let object = if is_super {
                        self.lookup(env, "\0super-base")?
                    } else {
                        self.expr(object, env)?
                    };
                    let key = self.expr(key, env)?;
                    let key = self.property_key(key)?;
                    let receiver = if is_super {
                        self.scope(env)?.this.clone()
                    } else {
                        object.clone()
                    };
                    let function = self.get_key_receiver(object.clone(), key, receiver)?;
                    (
                        function,
                        if is_super {
                            self.scope(env)?.this.clone()
                        } else {
                            object
                        },
                    )
                } else {
                    (self.expr(callee, env)?, JsValue::Undefined)
                };
                let args = self.arguments(args, env)?;
                if matches!(&callee.kind,ExprKind::Name(s)if s=="eval")
                    && function
                        .object()
                        .and_then(|id| self.heap.get(id).ok())
                        .is_some_and(|o| matches!(&o.kind, ObjectKind::Native(s) if s == "eval"))
                {
                    if let Some(JsValue::String(source)) = args.first() {
                        return self.dynamic_eval(source, Some(env));
                    }
                }
                if matches!(&callee.kind,ExprKind::Name(s)if s=="super") {
                    return self.call_inner(
                        function,
                        self.scope(env)?.this.clone(),
                        args,
                        true,
                        Some(env),
                    );
                }
                self.call_inner(function, this, args, false, None)
            }
            ExprKind::New(callee, args) => {
                let function = self.expr(callee, env)?;
                let args = self.arguments(args, env)?;
                self.construct(function, args)
            }
            ExprKind::Sequence(list) => {
                let mut last = JsValue::Undefined;
                for expr in list {
                    last = self.expr(expr, env)?;
                }
                Ok(last)
            }
            ExprKind::Conditional(test, yes, no) => {
                if self.expr(test, env)?.truthy() {
                    self.expr(yes, env)
                } else {
                    self.expr(no, env)
                }
            }
            ExprKind::Unary(op, operand) => {
                if op == "typeof" {
                    let value = match self.expr(operand, env) {
                        Ok(v) => v,
                        Err(e)
                            if e.kind == ErrorKind::ReferenceError
                                && match &operand.kind {
                                    ExprKind::Name(name) => !self.binding_exists(env, name)?,
                                    _ => false,
                                } =>
                        {
                            JsValue::Undefined
                        }
                        Err(e) => return Err(e),
                    };
                    return Ok(JsValue::string(self.typeof_value(&value)));
                }
                if op == "delete" {
                    if let ExprKind::Member(object, key) = &operand.kind {
                        let object = self.expr(object, env)?;
                        let key = self.expr(key, env)?;
                        let key = self.property_key(key)?;
                        return Ok(JsValue::Bool(self.delete(object, key)?));
                    }
                    if self.strict {
                        return Err(JsError::new(
                            ErrorKind::SyntaxError,
                            "delete identifier in strict mode",
                        ));
                    }
                    return Ok(JsValue::Bool(false));
                }
                let value = self.expr(operand, env)?;
                let value = if matches!(op.as_str(), "-" | "~") {
                    self.coerce_primitive(value, true)?
                } else {
                    value
                };
                if let JsValue::BigInt(n) = &value {
                    return match op.as_str() {
                        "-" => Ok(JsValue::BigInt(Arc::new(n.negated()))),
                        "~" => Ok(JsValue::BigInt(Arc::new(n.not()?))),
                        "!" => Ok(JsValue::Bool(!value.truthy())),
                        "void" => Ok(JsValue::Undefined),
                        _ => Err(JsError::new(
                            ErrorKind::TypeError,
                            "unary plus cannot convert BigInt",
                        )),
                    };
                }
                match op.as_str() {
                    "!" => Ok(JsValue::Bool(!value.truthy())),
                    "void" => Ok(JsValue::Undefined),
                    "+" => Ok(JsValue::Number(self.to_number(value)?)),
                    "-" => Ok(JsValue::Number(-self.to_number(value)?)),
                    "~" => Ok(JsValue::Number((!to_i32(self.to_number(value)?)) as f64)),
                    _ => Err(JsError::new(
                        ErrorKind::SyntaxError,
                        "unknown unary operator",
                    )),
                }
            }
            ExprKind::Update(op, target, prefix) => {
                let reference = self.reference(target, env)?;
                let old = self.read_reference(&reference, env)?;
                let old = self.coerce_primitive(old, true)?;
                if let JsValue::BigInt(n) = &old {
                    let value = if op == "++" {
                        n.add(&crate::bigint::BigInt::one())?
                    } else {
                        n.sub(&crate::bigint::BigInt::one())?
                    };
                    let value = JsValue::BigInt(Arc::new(value));
                    self.write_reference(reference, value.clone(), env)?;
                    return Ok(if *prefix { value } else { old });
                }
                let old = self.to_number(old)?;
                let new = JsValue::Number(old + if op == "++" { 1.0 } else { -1.0 });
                self.write_reference(reference, new.clone(), env)?;
                Ok(if *prefix { new } else { JsValue::Number(old) })
            }
            ExprKind::Binary(op, left, right) => {
                let left = self.expr(left, env)?;
                if op == "&&" && !left.truthy()
                    || op == "||" && left.truthy()
                    || op == "??" && !matches!(left, JsValue::Null | JsValue::Undefined)
                {
                    return Ok(left);
                }
                let right = self.expr(right, env)?;
                if matches!(op.as_str(), "&&" | "||" | "??") {
                    return Ok(right);
                }
                self.binary(op, left, right)
            }
            ExprKind::Assign(op, target, right) => {
                if matches!(target.kind, ExprKind::Array(_) | ExprKind::Object(_)) {
                    let value = self.expr(right, env)?;
                    self.assign_target(target, value.clone(), env)?;
                    return Ok(value);
                }
                let reference = self.reference(target, env)?;
                let left = if op == "=" {
                    None
                } else {
                    Some(self.read_reference(&reference, env)?)
                };
                let right = self.expr(right, env)?;
                let value = if op == "=" {
                    right
                } else {
                    self.binary(
                        op.trim_end_matches('='),
                        left.unwrap_or(JsValue::Undefined),
                        right,
                    )?
                };
                self.write_reference(reference, value.clone(), env)?;
                Ok(value)
            }
        }
    }
    fn reference(&mut self, target: &Expr, env: usize) -> Result<Reference, JsError> {
        match &target.kind {
            ExprKind::Name(name) => Ok(Reference::Name(name.clone())),
            ExprKind::Member(object, key) => {
                let object = self.expr(object, env)?;
                let key = self.expr(key, env)?;
                Ok(Reference::Property(object, self.property_key(key)?))
            }
            _ => Err(JsError::new(
                ErrorKind::ReferenceError,
                "invalid assignment target",
            )),
        }
    }
    fn read_reference(&mut self, r: &Reference, env: usize) -> Result<JsValue, JsError> {
        match r {
            Reference::Name(n) => self.lookup(env, n),
            Reference::Property(o, k) => self.get_key(o.clone(), k.clone()),
        }
    }
    fn write_reference(&mut self, r: Reference, v: JsValue, env: usize) -> Result<(), JsError> {
        match r {
            Reference::Name(n) => self.assign_name(env, &n, v),
            Reference::Property(o, k) => self.set_key(o, k, v),
        }
    }
    fn assign_target(&mut self, target: &Expr, value: JsValue, env: usize) -> Result<(), JsError> {
        match &target.kind {
            ExprKind::Name(name) => self.assign_name(env, name, value),
            ExprKind::Member(object, key) => {
                let object = self.expr(object, env)?;
                let key = self.expr(key, env)?;
                let key = self.property_key(key)?;
                self.set_key(object, key, value)
            }
            ExprKind::Array(parts) => {
                let values = self.array_values(value)?;
                for (i, p) in parts.iter().enumerate() {
                    if let Some(p) = p {
                        if p.spread {
                            let rest = self.array(values.iter().skip(i).cloned().collect())?;
                            self.assign_target(&p.expr, rest, env)?;
                        } else {
                            self.assign_target(
                                &p.expr,
                                values.get(i).cloned().unwrap_or(JsValue::Undefined),
                                env,
                            )?;
                        }
                    }
                }
                Ok(())
            }
            ExprKind::Object(props) => {
                for p in props {
                    if let ObjectProperty::Value(k, target) = p {
                        let key = self.prop_name(k, env)?;
                        let v = self.get_key(value.clone(), key)?;
                        self.assign_target(target, v, env)?;
                    }
                }
                Ok(())
            }
            _ => Err(JsError::new(
                ErrorKind::ReferenceError,
                "invalid assignment target",
            )),
        }
    }
    pub(crate) fn binary(
        &mut self,
        op: &str,
        left: JsValue,
        right: JsValue,
    ) -> Result<JsValue, JsError> {
        match op {
            "===" | "!==" => return Ok(JsValue::Bool(strict_equal(&left, &right) ^ (op == "!=="))),
            "==" | "!=" => return Ok(JsValue::Bool(self.equal(left, right)? ^ (op == "!="))),
            "in" => {
                let key = self.property_key(left)?;
                return Ok(JsValue::Bool(self.has(right, key)?));
            }
            "instanceof" => {
                if right.object().is_none() {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "instanceof rhs is not an object",
                    ));
                }
                let method = self.get_key(
                    right.clone(),
                    PropertyKey::Symbol(crate::symbols::HAS_INSTANCE),
                )?;
                if !matches!(method, JsValue::Undefined | JsValue::Null) {
                    if !self.callable(&method) {
                        return Err(JsError::new(
                            ErrorKind::TypeError,
                            "Symbol.hasInstance is not callable",
                        ));
                    }
                    return self
                        .call_inner(method, right, vec![left], false, None)
                        .map(|v| JsValue::Bool(v.truthy()));
                }
                if !self.callable(&right) {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "instanceof rhs is not callable",
                    ));
                }
                let proto = self.get(right, "prototype")?.object().ok_or_else(|| {
                    JsError::new(ErrorKind::TypeError, "invalid constructor prototype")
                })?;
                let mut current = left
                    .object()
                    .and_then(|id| self.heap.get(id).ok())
                    .and_then(|o| o.prototype);
                let mut seen = HashSet::new();
                while let Some(id) = current {
                    if id == proto {
                        return Ok(JsValue::Bool(true));
                    }
                    if !seen.insert(id) {
                        break;
                    }
                    current = self.heap.get(id)?.prototype;
                }
                return Ok(JsValue::Bool(false));
            }
            _ => {}
        }
        let hint = if op == "+" { "default" } else { "number" };
        let left = if matches!(left, JsValue::Object(_)) {
            self.coerce_primitive_hint(left, hint)?
        } else {
            left
        };
        let right = if matches!(right, JsValue::Object(_)) {
            self.coerce_primitive_hint(right, hint)?
        } else {
            right
        };
        if op == "+" && (matches!(left, JsValue::String(_)) || matches!(right, JsValue::String(_)))
        {
            let mut out = self.to_string(left)?;
            out.push_str(&self.to_string(right)?);
            if out.len() > self.limits.max_string_bytes {
                return Err(JsError::new(ErrorKind::MemoryLimit, "string limit"));
            }
            return Ok(JsValue::string(out));
        }
        if matches!(op, "<" | ">" | "<=" | ">=") {
            let comparison = match (&left, &right) {
                (JsValue::BigInt(a), JsValue::BigInt(b)) => Some(Some(a.compare(b))),
                (JsValue::BigInt(a), JsValue::Number(b)) => Some(a.compare_number(*b)),
                (JsValue::Number(a), JsValue::BigInt(b)) => {
                    Some(b.compare_number(*a).map(std::cmp::Ordering::reverse))
                }
                (JsValue::BigInt(a), JsValue::String(b)) => {
                    Some(crate::bigint::BigInt::parse(b).ok().map(|b| a.compare(&b)))
                }
                (JsValue::String(a), JsValue::BigInt(b)) => {
                    Some(crate::bigint::BigInt::parse(a).ok().map(|a| a.compare(b)))
                }
                (JsValue::BigInt(a), value) => {
                    Some(a.compare_number(self.to_number(value.clone())?))
                }
                (value, JsValue::BigInt(b)) => Some(
                    b.compare_number(self.to_number(value.clone())?)
                        .map(std::cmp::Ordering::reverse),
                ),
                _ => None,
            };
            if let Some(order) = comparison {
                return Ok(JsValue::Bool(order.is_some_and(|c| match op {
                    "<" => c.is_lt(),
                    ">" => c.is_gt(),
                    "<=" => !c.is_gt(),
                    _ => !c.is_lt(),
                })));
            }
            if let (JsValue::String(a), JsValue::String(b)) = (&left, &right) {
                let cmp = a.encode_utf16().cmp(b.encode_utf16());
                return Ok(JsValue::Bool(match op {
                    "<" => cmp.is_lt(),
                    ">" => cmp.is_gt(),
                    "<=" => !cmp.is_gt(),
                    _ => !cmp.is_lt(),
                }));
            }
        }
        if let (JsValue::BigInt(a), JsValue::BigInt(b)) = (&left, &right) {
            let out = match op {
                "+" => a.add(b)?,
                "-" => a.sub(b)?,
                "*" => a.mul(b)?,
                "/" => a.div_rem(b)?.0,
                "%" => a.div_rem(b)?.1,
                "**" => a.pow(b)?,
                "&" | "|" | "^" => a.bitwise(b, op)?,
                "<<" => a.shift(b, true)?,
                ">>" => a.shift(b, false)?,
                _ => return Err(JsError::new(ErrorKind::TypeError, "BigInt operator")),
            };
            return Ok(JsValue::BigInt(Arc::new(out)));
        }
        if matches!(left, JsValue::BigInt(_)) || matches!(right, JsValue::BigInt(_)) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "cannot mix BigInt and Number",
            ));
        }
        let a = self.to_number(left)?;
        let b = self.to_number(right)?;
        Ok(match op {
            "+" => JsValue::Number(a + b),
            "-" => JsValue::Number(a - b),
            "*" => JsValue::Number(a * b),
            "/" => JsValue::Number(a / b),
            "%" => JsValue::Number(a % b),
            "**" => JsValue::Number(a.powf(b)),
            "<" => JsValue::Bool(a < b),
            ">" => JsValue::Bool(a > b),
            "<=" => JsValue::Bool(a <= b),
            ">=" => JsValue::Bool(a >= b),
            "&" => JsValue::Number((to_i32(a) & to_i32(b)) as f64),
            "|" => JsValue::Number((to_i32(a) | to_i32(b)) as f64),
            "^" => JsValue::Number((to_i32(a) ^ to_i32(b)) as f64),
            "<<" => JsValue::Number(to_i32(a).wrapping_shl(to_i32(b) as u32 & 31) as f64),
            ">>" => JsValue::Number((to_i32(a) >> (to_i32(b) as u32 & 31)) as f64),
            ">>>" => JsValue::Number(((to_i32(a) as u32) >> (to_i32(b) as u32 & 31)) as f64),
            _ => {
                return Err(JsError::new(
                    ErrorKind::SyntaxError,
                    "unknown binary operator",
                ))
            }
        })
    }
    fn equal(&mut self, a: JsValue, b: JsValue) -> Result<bool, JsError> {
        if strict_equal(&a, &b) {
            return Ok(true);
        }
        if matches!(
            (&a, &b),
            (JsValue::Null, JsValue::Undefined) | (JsValue::Undefined, JsValue::Null)
        ) {
            return Ok(true);
        }
        match (&a, &b) {
            (JsValue::BigInt(x), JsValue::String(y)) | (JsValue::String(y), JsValue::BigInt(x)) => {
                Ok(crate::bigint::BigInt::parse(y).is_ok_and(|y| x.as_ref() == &y))
            }
            (JsValue::BigInt(x), JsValue::Number(y)) | (JsValue::Number(y), JsValue::BigInt(x)) => {
                Ok(x.compare_number(*y) == Some(std::cmp::Ordering::Equal))
            }
            (JsValue::BigInt(_), JsValue::Object(_)) => {
                let b = self.coerce_primitive_hint(b, "default")?;
                self.equal(a, b)
            }
            (JsValue::Object(_), JsValue::BigInt(_)) => {
                let a = self.coerce_primitive_hint(a, "default")?;
                self.equal(a, b)
            }
            (JsValue::Bool(value), _) => {
                self.equal(JsValue::Number(if *value { 1.0 } else { 0.0 }), b)
            }
            (_, JsValue::Bool(value)) => {
                self.equal(a, JsValue::Number(if *value { 1.0 } else { 0.0 }))
            }
            (JsValue::Number(_), JsValue::String(_)) | (JsValue::String(_), JsValue::Number(_)) => {
                Ok(self.to_number(a)? == self.to_number(b)?)
            }
            (JsValue::Object(_), JsValue::String(_) | JsValue::Number(_) | JsValue::Symbol(..)) => {
                let a = self.coerce_primitive_hint(a, "default")?;
                self.equal(a, b)
            }
            (JsValue::String(_) | JsValue::Number(_) | JsValue::Symbol(..), JsValue::Object(_)) => {
                let b = self.coerce_primitive_hint(b, "default")?;
                self.equal(a, b)
            }
            _ => Ok(false),
        }
    }
    pub fn has(&mut self, value: JsValue, key: PropertyKey) -> Result<bool, JsError> {
        let Some(mut id) = value.object() else {
            return Err(JsError::new(ErrorKind::TypeError, "in requires object"));
        };
        let mut seen = HashSet::new();
        while seen.insert(id) {
            let object = self.heap.get(id)?;
            if let ObjectKind::ModuleNamespace(module) = object.kind {
                if let PropertyKey::String(name) = &key {
                    return Ok(self.module_names(module)?.contains(name));
                }
            }
            if matches!(object.kind, ObjectKind::Proxy(_)) {
                return self.proxy_has(id, key);
            }
            if object.properties.contains_key(&key) {
                return Ok(true);
            }
            if let Some(proto) = object.prototype {
                id = proto
            } else {
                break;
            }
        }
        Ok(false)
    }
    fn enumerable_keys(&mut self, value: JsValue) -> Result<Vec<PropertyKey>, JsError> {
        let mut current = value.object();
        let mut seen = HashSet::new();
        let mut keys = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                break;
            }
            keys.extend(self.keys(&JsValue::Object(id), true)?);
            current = self.heap.get(id)?.prototype;
        }
        Ok(keys.into_iter().collect())
    }
    pub fn typeof_value(&self, value: &JsValue) -> &'static str {
        match value {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object",
            JsValue::Bool(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::BigInt(_) => "bigint",
            JsValue::String(_) => "string",
            JsValue::Symbol(..) => "symbol",
            JsValue::Object(_) => {
                if self.callable(value) {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }
    fn construct(&mut self, function: JsValue, args: Vec<JsValue>) -> Result<JsValue, JsError> {
        let id = function
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "new target is not constructor"))?;
        let kind = self.heap.get(id)?.kind.clone();
        if let ObjectKind::Proxy(data) = &kind {
            if !data.constructible {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "Proxy target is not a constructor",
                ));
            }
            return self.proxy_call(id, JsValue::Undefined, args, true);
        }
        if let ObjectKind::Function(f) = &kind {
            if !f.constructable {
                return Err(JsError::new(
                    ErrorKind::TypeError,
                    "arrow is not constructor",
                ));
            }
        }
        if let ObjectKind::Native(name) = kind {
            if name.starts_with("host:") {
                let prototype = self.get(function.clone(), "prototype")?;
                let value = self.call_inner(function, JsValue::Undefined, args, true, None)?;
                self.set_host_prototype(&value, prototype)?;
                return Ok(value);
            }
            return crate::builtins::call(self, &name, JsValue::Undefined, args, true, None);
        }
        let prototype = self
            .get(function.clone(), "prototype")?
            .object()
            .unwrap_or(self.object_proto);
        let object = JsValue::Object(self.allocate(ObjectKind::Ordinary, Some(prototype))?);
        let result = self.call_inner(function, object.clone(), args, true, None)?;
        Ok(if matches!(result, JsValue::Object(_)) {
            result
        } else {
            object
        })
    }
    pub(crate) fn call_inner(
        &mut self,
        function: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
        construct: bool,
        direct_env: Option<usize>,
    ) -> Result<JsValue, JsError> {
        self.tick()?;
        if self.depth >= self.limits.max_call_depth {
            return Err(JsError::new(
                ErrorKind::RangeError,
                "maximum call stack exceeded",
            ));
        }
        let id = function
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "value is not callable"))?;
        let kind = self.heap.get(id)?.kind.clone();
        self.depth += 1;
        let result = (|| match kind {
            ObjectKind::Proxy(_) => self.proxy_call(id, this, args, construct),
            ObjectKind::ProxyRevoker(proxy) => {
                if let Ok(Object {
                    kind: ObjectKind::Proxy(data),
                    ..
                }) = self.heap.get_mut(proxy)
                {
                    data.handler = None;
                }
                Ok(JsValue::Undefined)
            }
            ObjectKind::PromiseClosure(closure) => {
                if construct {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "promise callback is not a constructor",
                    ));
                }
                self.call_promise_closure(*closure, args)
            }
            ObjectKind::Native(name) => {
                if name.starts_with("host:") {
                    let large = args
                        .iter()
                        .any(|v| matches!(v, JsValue::String(s) if s.len() > 4096));
                    let outcome = self
                        .with_host(|host, vm| host.call(vm, &name[5..], this, args.as_slice()))?;
                    self.with_host(|host, vm| host.after_operation(vm))?;
                    self.checkpoint()?;
                    if large
                        || name.starts_with("host:web:")
                        || name == "host:postMessage"
                        || matches!(
                            name.as_str(),
                            "host:cloneNode"
                                | "host:append"
                                | "host:prepend"
                                | "host:insertAdjacentHTML"
                        )
                    {
                        self.finish_memory_check(JsValue::Undefined)?;
                    }
                    for (f, t, a) in outcome.calls {
                        let allowed =
                            self.with_host(|host, vm| Ok(host.before_callback(vm, &f, &t, &a)))?;
                        if allowed {
                            let result = self.call_inner(f, t, a, false, None);
                            if let Err(error) = result {
                                if error.is_resource_limit() || !outcome.catch_callback_errors {
                                    return Err(error);
                                }
                                let message = error.to_string();
                                self.emit_console(&message)?;
                                if self.console.len() < 1000 {
                                    self.console.push(message.chars().take(16_384).collect());
                                }
                            }
                        }
                    }
                    if let Some(token) = outcome.finish {
                        self.with_host(|host, vm| host.finish(vm, token))
                    } else {
                        Ok(outcome.value)
                    }
                } else {
                    crate::builtins::call(self, &name, this, args, construct, direct_env)
                }
            }
            ObjectKind::Bound {
                function,
                this: bound,
                args: mut prefix,
            } => {
                prefix.extend(args);
                self.call_inner(
                    function,
                    if construct { this } else { bound },
                    prefix,
                    construct,
                    None,
                )
            }
            ObjectKind::Function(f) => {
                if f.class && !construct {
                    Err(JsError::new(ErrorKind::TypeError, "class requires new"))
                } else {
                    let old = self.strict;
                    let source = std::mem::replace(&mut self.source_url, f.source_url.clone());
                    let module = self.current_module;
                    self.current_module = f.module;
                    let active_before = self.active.len();
                    self.strict = f.strict;
                    let result = (|| {
                        let this = f.lexical_this.clone().unwrap_or({
                            if !f.strict && matches!(this, JsValue::Undefined | JsValue::Null) {
                                JsValue::Object(self.global)
                            } else if !f.strict && !matches!(this, JsValue::Object(_)) {
                                let prototype = match this {
                                    JsValue::String(_) => self.string_proto,
                                    JsValue::Number(_) => self.number_proto,
                                    JsValue::Bool(_) => self.boolean_proto,
                                    _ => self.object_proto,
                                };
                                JsValue::Object(
                                    self.allocate(ObjectKind::Boxed(this), Some(prototype))?,
                                )
                            } else {
                                this
                            }
                        });
                        let env = self.env(f.environment, true, this.clone())?;
                        self.active.push(env);
                        if let Some(name) = &f.function.name {
                            self.declare_name(env, name, DeclKind::Var, function.clone(), true)?;
                        }
                        if let Some(super_id) = f.super_constructor {
                            self.declare_name(
                                env,
                                "super",
                                DeclKind::Var,
                                JsValue::Object(super_id),
                                true,
                            )?;
                        }
                        if let Some(base) = f.super_base {
                            self.declare_name(
                                env,
                                "\0super-base",
                                DeclKind::Var,
                                JsValue::Object(base),
                                true,
                            )?;
                            if f.super_constructor.is_none() {
                                self.declare_name(
                                    env,
                                    "super",
                                    DeclKind::Var,
                                    JsValue::Object(base),
                                    true,
                                )?;
                            }
                        }
                        for (i, param) in f.function.params.iter().enumerate() {
                            let value = if matches!(param, Pattern::Rest(_)) {
                                self.array(args.iter().skip(i).cloned().collect())?
                            } else {
                                args.get(i).cloned().unwrap_or(JsValue::Undefined)
                            };
                            self.bind(param, value, env, DeclKind::Var)?;
                        }
                        if !f.function.arrow {
                            let arguments = self.array(args.clone())?;
                            self.declare_name(env, "arguments", DeclKind::Var, arguments, true)?;
                        }
                        if construct && f.class && f.function.body.is_empty() {
                            if let Some(super_id) = f.super_constructor {
                                self.call_inner(JsValue::Object(super_id), this, args, true, None)?;
                            }
                        }
                        if !self.ast_reference {
                            return self.execute_function(&f, env);
                        }
                        self.hoist(&f.function.body, env, true)?;
                        let result = match self.statements(&f.function.body, env) {
                            Ok(Completion::Return(v)) => Ok(v),
                            Ok(_) => Ok(JsValue::Undefined),
                            Err(mut e) => {
                                e.stack.push(format!(
                                    "{} ({}:{})",
                                    f.function.name.as_deref().unwrap_or("<anonymous>"),
                                    f.function.span.line,
                                    f.function.span.column
                                ));
                                Err(e)
                            }
                        };
                        result
                    })();
                    self.active.truncate(active_before);
                    self.strict = old;
                    self.source_url = source;
                    self.current_module = module;
                    match result {
                        Err(error)
                            if f.function.asynchronous
                                && !f.function.generator
                                && !error.is_resource_limit() =>
                        {
                            let promise = self.promise()?;
                            let reason = if let Some(value) = error.thrown {
                                value
                            } else {
                                self.error_value(error)?
                            };
                            self.reject_promise(promise.clone(), reason)?;
                            Ok(promise)
                        }
                        result => result,
                    }
                }
            }
            _ => Err(JsError::new(ErrorKind::TypeError, "value is not callable")),
        })();
        self.depth = self.depth.saturating_sub(1);
        result.and_then(|value| {
            self.check_value(&value)?;
            Ok(value)
        })
    }
    pub(crate) fn dynamic_eval(
        &mut self,
        source: &str,
        direct_env: Option<usize>,
    ) -> Result<JsValue, JsError> {
        if !self
            .host
            .as_ref()
            .map_or(self.dynamic_code_allowed, |h| h.allow_dynamic_code())
        {
            return Err(JsError::new(ErrorKind::Error, "CSP blocks dynamic code"));
        }
        let program = parse(source, ParseOpts::default())
            .map_err(|e| JsError::new(ErrorKind::SyntaxError, e.to_string()))?;
        let old = self.strict;
        self.strict = program.strict || (direct_env.is_some() && old);
        let env = direct_env.unwrap_or(0);
        let env = if self.strict {
            self.env(env, true, self.scope(env)?.this.clone())?
        } else {
            env
        };
        let result = self.eval_body(&program, env);
        self.strict = old;
        result
    }
    pub(crate) fn regexp(&mut self, source: &str, flags: &str) -> Result<JsValue, JsError> {
        let mut seen = HashSet::new();
        for flag in flags.chars() {
            if !"gimyus".contains(flag) || !seen.insert(flag) {
                return Err(JsError::new(ErrorKind::SyntaxError, "invalid regexp flags"));
            }
        }
        let regex = regex::RegexBuilder::new(source)
            .case_insensitive(flags.contains('i'))
            .multi_line(flags.contains('m'))
            .dot_matches_new_line(flags.contains('s'))
            .size_limit(2 * 1024 * 1024)
            .dfa_size_limit(10 * 1024 * 1024)
            .build()
            .map_err(|e| JsError::new(ErrorKind::SyntaxError, e.to_string()))?;
        let id = self.allocate(
            ObjectKind::RegExp {
                regex,
                source: source.into(),
                flags: flags.into(),
            },
            Some(self.regexp_proto),
        )?;
        self.define(id, "lastIndex".into(), Property::data(JsValue::Number(0.0)))?;
        self.define(id, "source".into(), Property::data(JsValue::string(source)))?;
        for (name, flag) in [("global", 'g'), ("ignoreCase", 'i'), ("multiline", 'm')] {
            self.define(
                id,
                name.into(),
                Property::data(JsValue::Bool(flags.contains(flag))),
            )?;
        }
        Ok(JsValue::Object(id))
    }
    pub fn error_value(&mut self, error: JsError) -> Result<JsValue, JsError> {
        let stack = if error.stack.is_empty() {
            self.current_stack()
        } else {
            error.stack.clone()
        };
        let id = self.allocate(ObjectKind::Error, Some(self.error_proto))?;
        self.define(
            id,
            "name".into(),
            Property::data(JsValue::string(format!("{:?}", error.kind))),
        )?;
        self.define(
            id,
            "message".into(),
            Property::data(JsValue::string(error.message)),
        )?;
        self.define(
            id,
            "stack".into(),
            Property {
                enumerable: false,
                ..Property::data(JsValue::string(stack.join("\n")))
            },
        )?;
        Ok(JsValue::Object(id))
    }
    pub fn pin(&mut self, value: JsValue) -> usize {
        let id = self.pins.len();
        self.pins.push(value);
        id
    }
    pub fn unpin(&mut self, id: usize) {
        if let Some(v) = self.pins.get_mut(id) {
            *v = JsValue::Undefined;
        }
    }
    pub fn heap_objects(&self) -> usize {
        self.heap.live()
    }
    pub fn gc_collect(&mut self) {
        // Rust locals of an executing host/activation are not arena roots. Honor
        // reentrant requests at the next checkpoint after the JS stack empties.
        if self.depth != 0 || !self.call_stack.is_empty() {
            self.gc_requested = true;
            return;
        }
        self.gc_requested = false;
        enum Mark {
            Value(JsValue),
            Env(usize),
        }
        #[derive(PartialEq, Eq, Hash)]
        enum WeakKey {
            Object(ObjectId),
            Symbol(u64),
        }
        let mut ephemerons: HashMap<WeakKey, Vec<JsValue>> = HashMap::new();
        let mut marked = HashSet::new();
        let mut envs = HashSet::new();
        let mut hosts = HashSet::new();
        let candidates = self.host_objects.keys().copied().collect::<HashSet<_>>();
        let mut symbols = HashSet::new();
        symbols.extend(1..=crate::symbols::ASYNC_ITERATOR);
        let mut work = vec![Mark::Env(0), Mark::Value(self.last.clone())];
        for job in &self.microtasks {
            if let promises::Job::Host(token) = job {
                hosts.insert(*token);
            }
        }
        work.extend(self.pins.iter().cloned().map(Mark::Value));
        work.extend(
            self.async_errors
                .iter()
                .filter_map(|error| error.thrown.clone())
                .map(Mark::Value),
        );
        work.extend(self.active.iter().copied().map(Mark::Env));
        work.extend(
            self.modules
                .iter()
                .map(|module| Mark::Env(module.environment)),
        );
        work.extend(self.module_roots().into_iter().map(Mark::Value));
        work.extend(self.module_loads.values().cloned().map(Mark::Value));
        work.extend(
            self.collection_protos
                .into_iter()
                .map(|id| Mark::Value(JsValue::Object(id))),
        );
        for id in [
            self.object_proto,
            self.function_proto,
            self.array_proto,
            self.string_proto,
            self.number_proto,
            self.boolean_proto,
            self.date_proto,
            self.regexp_proto,
            self.error_proto,
            self.promise_proto,
            self.promise_ctor,
            self.aggregate_error_proto,
            self.iterator_proto,
            self.bigint_proto,
            self.generator_proto,
            self.async_generator_proto,
            self.symbol_proto,
        ] {
            work.push(Mark::Value(JsValue::Object(id)));
        }
        work.extend(
            self.microtasks
                .iter()
                .flat_map(promises::Job::roots)
                .map(Mark::Value),
        );
        work.extend(
            self.rejected_promises
                .iter()
                .copied()
                .map(|id| Mark::Value(JsValue::Object(id))),
        );
        work.extend(
            self.promise_notifications
                .iter()
                .flat_map(|n| [n.promise.clone(), n.reason.clone()])
                .map(Mark::Value),
        );
        loop {
            while let Some(item) = work.pop() {
                match item {
                    Mark::Env(id) => {
                        if !envs.insert(id) {
                            continue;
                        }
                        if let Ok(e) = self.scope(id) {
                            if let Some(parent) = e.parent {
                                work.push(Mark::Env(parent));
                            }
                            work.push(Mark::Value(e.this.clone()));
                            work.extend(e.aliases.values().map(|(env, _)| Mark::Env(*env)));
                            work.extend(e.bindings.values().map(|b| Mark::Value(b.value.clone())));
                        }
                    }
                    Mark::Value(JsValue::Object(id)) => {
                        if !marked.insert(id) {
                            continue;
                        }
                        if let Some(values) = ephemerons.remove(&WeakKey::Object(id)) {
                            work.extend(values.into_iter().map(Mark::Value));
                        }
                        let Ok(object) = self.heap.get(id) else {
                            continue;
                        };
                        if let Some(p) = object.prototype {
                            work.push(Mark::Value(JsValue::Object(p)));
                        }
                        for key in object.properties.keys() {
                            if let PropertyKey::Symbol(id) = key {
                                work.push(Mark::Value(JsValue::Symbol(*id, "".into())));
                            }
                        }
                        for p in object.properties.values() {
                            match &p.value {
                                PropertyValue::Data(v) => work.push(Mark::Value(v.clone())),
                                PropertyValue::Accessor { get, set } => {
                                    work.push(Mark::Value(get.clone()));
                                    work.push(Mark::Value(set.clone()));
                                }
                            }
                        }
                        match &object.kind {
                            ObjectKind::Proxy(data) => {
                                if let Some(handler) = &data.handler {
                                    work.push(Mark::Value(handler.clone()));
                                    work.push(Mark::Value(JsValue::Object(data.target)));
                                }
                            }
                            ObjectKind::ProxyRevoker(id) => {
                                work.push(Mark::Value(JsValue::Object(*id)))
                            }
                            ObjectKind::Collection(data) => {
                                if data.kind == collections::CollectionKind::WeakMap {
                                    // Index suspended ephemeron edges by their key; each
                                    // newly marked key activates its edges only once.
                                    for (key, value) in data.entries.values() {
                                        if value.object().is_none()
                                            && !matches!(value, JsValue::Symbol(..))
                                        {
                                            continue;
                                        }
                                        let (key, live) = match key {
                                            JsValue::Object(id) => {
                                                (WeakKey::Object(*id), marked.contains(id))
                                            }
                                            JsValue::Symbol(id, _) => {
                                                (WeakKey::Symbol(*id), symbols.contains(id))
                                            }
                                            _ => continue,
                                        };
                                        if live {
                                            work.push(Mark::Value(value.clone()));
                                        } else {
                                            ephemerons.entry(key).or_default().push(value.clone());
                                        }
                                    }
                                } else {
                                    work.extend(data.roots().into_iter().map(Mark::Value));
                                }
                            }
                            ObjectKind::AsyncTask(data) => {
                                work.extend(data.roots().into_iter().map(Mark::Value));
                                work.extend(data.environments().into_iter().map(Mark::Env));
                            }
                            ObjectKind::Generator(data) => {
                                work.extend(data.roots().into_iter().map(Mark::Value));
                                work.extend(data.environments().into_iter().map(Mark::Env));
                            }
                            ObjectKind::Promise(data) => {
                                work.extend(data.roots().into_iter().map(Mark::Value))
                            }
                            ObjectKind::PromiseClosure(data) => {
                                work.extend(data.roots().into_iter().map(Mark::Value))
                            }
                            ObjectKind::PromiseInternal(data) => {
                                work.extend(data.roots().into_iter().map(Mark::Value))
                            }
                            ObjectKind::Iterator(data) => {
                                work.extend(data.roots().into_iter().map(Mark::Value))
                            }
                            ObjectKind::Function(f) => {
                                work.push(Mark::Env(f.environment));
                                if let Some(v) = &f.lexical_this {
                                    work.push(Mark::Value(v.clone()));
                                }
                                if let Some(v) = f.super_constructor {
                                    work.push(Mark::Value(JsValue::Object(v)));
                                }
                                if let Some(v) = f.super_base {
                                    work.push(Mark::Value(JsValue::Object(v)));
                                }
                            }
                            ObjectKind::Bound {
                                function,
                                this,
                                args,
                            } => {
                                work.push(Mark::Value(function.clone()));
                                work.push(Mark::Value(this.clone()));
                                work.extend(args.iter().cloned().map(Mark::Value));
                            }
                            ObjectKind::Boxed(v) => work.push(Mark::Value(v.clone())),
                            ObjectKind::Host { id, .. } => {
                                hosts.insert(*id);
                            }
                            _ => {}
                        }
                    }
                    Mark::Value(JsValue::Symbol(id, _)) if symbols.insert(id) => {
                        if let Some(values) = ephemerons.remove(&WeakKey::Symbol(id)) {
                            work.extend(values.into_iter().map(Mark::Value));
                        }
                    }
                    _ => {}
                }
            }
            let references = self.host.as_ref().map_or_else(Vec::new, |h| {
                h.host_references(&hosts)
                    .into_iter()
                    .chain(h.trace_native_edges(&hosts, &candidates))
                    .collect::<Vec<_>>()
            });
            let mut added = false;
            for reference in references {
                if hosts.insert(reference) {
                    added = true;
                    if let Some(id) = self.host_objects.get(&reference) {
                        work.push(Mark::Value(JsValue::Object(*id)));
                    }
                }
            }
            let extra = self
                .host
                .as_ref()
                .map_or_else(Vec::new, |h| h.roots(&hosts));
            for value in extra {
                if match &value {
                    JsValue::Object(id) => !marked.contains(id),
                    JsValue::Symbol(id, _) => !symbols.contains(id),
                    _ => false,
                } {
                    work.push(Mark::Value(value));
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        for (i, slot) in self.heap.slots.iter_mut().enumerate() {
            if let Some(Object {
                kind: ObjectKind::Collection(data),
                ..
            }) = &mut slot.object
            {
                data.sweep_weak(&marked, &symbols);
            }
            let id = ObjectId {
                index: i,
                generation: slot.generation,
            };
            if slot.object.is_some() && !marked.contains(&id) {
                slot.object = None;
                self.heap.free.push(i);
            }
        }
        for (i, environment) in self.environments.iter_mut().enumerate() {
            if environment.is_some() && !envs.contains(&i) {
                *environment = None;
                self.free_env.push(i);
            }
        }
        for value in self.symbol_registry.values() {
            if let JsValue::Symbol(id, _) = value {
                symbols.insert(*id);
            }
        }
        self.symbol_descriptions
            .retain(|id, _| symbols.contains(id));
        self.host_objects.retain(|_, id| marked.contains(id));
        if let Some(host) = &mut self.host {
            host.sweep(&hosts);
        }
        self.memory_bytes = self
            .heap
            .slots
            .iter()
            .filter_map(|s| s.object.as_ref())
            .fold(0usize, |n, o| n.saturating_add(o.retained_bytes()));
        for env in self.environments.iter().flatten() {
            self.memory_bytes = self
                .memory_bytes
                .saturating_add(256 + env.this.retained_bytes());
            self.memory_bytes = self.memory_bytes.saturating_add(
                env.aliases
                    .iter()
                    .map(|(key, (_, name))| key.len() + name.len() + 128)
                    .sum::<usize>(),
            );
            for (name, binding) in &env.bindings {
                self.memory_bytes = self
                    .memory_bytes
                    .saturating_add(192 + name.len() + binding.value.retained_bytes());
            }
        }
        self.memory_bytes = self.memory_bytes.saturating_add(
            self.microtasks
                .iter()
                .map(promises::Job::retained_bytes)
                .sum::<usize>(),
        );
        self.memory_bytes = self.memory_bytes.saturating_add(
            self.modules
                .iter()
                .map(|module| module.bytes)
                .sum::<usize>(),
        );
        self.memory_bytes = self
            .memory_bytes
            .saturating_add(
                self.symbol_descriptions
                    .values()
                    .map(|s| 64 + s.as_ref().map_or(0, String::len))
                    .sum::<usize>(),
            )
            .saturating_add(
                self.symbol_registry
                    .keys()
                    .map(|s| s.len() + 96)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.module_urls
                    .keys()
                    .chain(self.module_loads.keys())
                    .map(|s| s.len() + 64)
                    .sum::<usize>(),
            );
    }
    pub(crate) fn current_stack(&self) -> Vec<String> {
        self.call_stack
            .iter()
            .rev()
            .map(|(name, url, span)| {
                format!(
                    "at {} ({}:{}:{})",
                    if name.is_empty() { "<script>" } else { name },
                    url,
                    span.line,
                    span.column
                )
            })
            .collect()
    }
}
pub(crate) fn strict_equal(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Number(a), JsValue::Number(b)) => a == b,
        (JsValue::Symbol(a, _), JsValue::Symbol(b, _)) => a == b,
        _ => a == b,
    }
}
pub(crate) fn number_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n == f64::INFINITY {
        "Infinity".into()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".into()
    } else if n == 0.0 {
        "0".into()
    } else {
        n.to_string()
    }
}
pub(crate) fn to_i32(n: f64) -> i32 {
    if !n.is_finite() || n == 0.0 {
        0
    } else {
        n.trunc().rem_euclid(4294967296.0) as u32 as i32
    }
}
