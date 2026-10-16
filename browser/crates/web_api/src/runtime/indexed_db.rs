//! Atomic copy-on-write IndexedDB subset. No page value ever becomes a file path.
use super::*;
use crate::storage::{Database, Index, ObjectStore};
use dom_bindings::js::value::Property;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Active,
    Done,
    Aborted,
}
#[derive(Clone)]
pub(crate) struct Transaction {
    connection: u64,
    name: String,
    mode: String,
    scope: Vec<String>,
    phase: Phase,
    data: Arc<Database>,
    pending: usize,
    dirty: bool,
    open: Option<u64>,
    committing: bool,
}
#[derive(Clone)]
pub(crate) enum Idb {
    Connection {
        name: String,
        version: u64,
        closed: bool,
        upgrade: Option<u64>,
    },
    Transaction(Transaction),
    Store {
        transaction: u64,
        name: String,
    },
    Index {
        transaction: u64,
        store: String,
        name: String,
    },
    Request {
        result: JsValue,
        error: JsValue,
        done: bool,
        source: Option<u64>,
        transaction: Option<u64>,
    },
}
impl Idb {
    pub fn roots(&self) -> Vec<JsValue> {
        match self {
            Self::Request { result, error, .. } => vec![result.clone(), error.clone()],
            _ => Vec::new(),
        }
    }
    pub fn references(&self) -> Vec<u64> {
        match self {
            Self::Connection { upgrade, .. } => upgrade.iter().copied().collect(),
            Self::Transaction(t) => std::iter::once(t.connection).chain(t.open).collect(),
            Self::Store { transaction, .. } | Self::Index { transaction, .. } => vec![*transaction],
            Self::Request {
                source,
                transaction,
                ..
            } => source.iter().chain(transaction).copied().collect(),
        }
    }
    pub fn retained_bytes(&self) -> usize {
        match self {
            Self::Transaction(t) => t
                .data
                .stores
                .values()
                .map(|s| {
                    s.records
                        .iter()
                        .map(|(k, v)| k.len() + json_bytes(v))
                        .sum::<usize>()
                        + s.indexes
                            .values()
                            .map(|i| {
                                i.values
                                    .iter()
                                    .map(|(k, v)| {
                                        k.len() + v.iter().map(String::len).sum::<usize>()
                                    })
                                    .sum::<usize>()
                            })
                            .sum::<usize>()
                })
                .sum(),
            _ => 256,
        }
    }
}
fn json_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::String(s) => s.len() + 32,
        serde_json::Value::Array(a) => a.iter().map(json_bytes).sum::<usize>() + a.len() * 32,
        serde_json::Value::Object(m) => m.iter().map(|(k, v)| k.len() + json_bytes(v) + 64).sum(),
        _ => 32,
    }
}
#[derive(Clone)]
pub(crate) enum Task {
    Open {
        request: u64,
        name: String,
        version: Option<u64>,
        delete: bool,
    },
    Start {
        transaction: u64,
    },
    Operation {
        request: u64,
        transaction: u64,
        store: String,
        index: Option<String>,
        operation: String,
        key: Option<String>,
        value: Option<CloneData>,
        count: usize,
    },
    Commit {
        transaction: u64,
    },
}
impl Task {
    pub fn references(&self) -> Vec<u64> {
        match self {
            Self::Open { request, .. } => vec![*request],
            Self::Start { transaction } | Self::Commit { transaction } => vec![*transaction],
            Self::Operation {
                request,
                transaction,
                ..
            } => vec![*request, *transaction],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum Key {
    Number(f64),
    Date(f64),
    String(String),
    Array(Vec<Key>),
}
impl Key {
    fn rank(&self) -> u8 {
        match self {
            Self::Number(_) => 0,
            Self::Date(_) => 1,
            Self::String(_) => 2,
            Self::Array(_) => 3,
        }
    }
    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Self::Number(a), Self::Number(b)) | (Self::Date(a), Self::Date(b)) => a.total_cmp(b),
            (Self::String(a), Self::String(b)) => a.encode_utf16().cmp(b.encode_utf16()),
            (Self::Array(a), Self::Array(b)) => {
                for (a, b) in a.iter().zip(b) {
                    let c = a.compare(b);
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                a.len().cmp(&b.len())
            }
            _ => self.rank().cmp(&other.rank()),
        }
    }
    fn js(&self, vm: &mut Vm) -> std::result::Result<JsValue, JsError> {
        match self {
            Self::Number(n) | Self::Date(n) => Ok(JsValue::Number(*n)),
            Self::String(s) => Ok(JsValue::string(s)),
            Self::Array(values) => {
                let mut out = Vec::new();
                for value in values {
                    out.push(value.js(vm)?);
                }
                vm.array(out)
            }
        }
    }
}
fn key(
    vm: &mut Vm,
    value: JsValue,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> std::result::Result<Key, JsError> {
    vm.checkpoint()?;
    if depth > 32 || *nodes >= 1024 {
        return Err(error(vm, ApiError::new("DataError", "key depth")));
    }
    *nodes += 1;
    match value {
        JsValue::Number(n) if n.is_finite() => Ok(Key::Number(if n == 0.0 { 0.0 } else { n })),
        JsValue::String(s) => {
            if bytes.saturating_add(s.len()) > 65536 {
                return Err(error(vm, ApiError::new("DataError", "key byte quota")));
            }
            *bytes += s.len();
            Ok(Key::String(s.to_string()))
        }
        v if vm.date_value(&v).is_some_and(f64::is_finite) => {
            Ok(Key::Date(vm.date_value(&v).unwrap_or(0.0)))
        }
        v if vm.is_array(&v) => {
            let values = vm.array_values(v)?;
            if values.len() > 1024 {
                return Err(error(vm, ApiError::new("DataError", "key array quota")));
            }
            let mut out = Vec::new();
            for value in values {
                out.push(key(vm, value, depth + 1, nodes, bytes)?);
            }
            Ok(Key::Array(out))
        }
        _ => Err(error(
            vm,
            ApiError::new("DataError", "invalid database key"),
        )),
    }
}
fn encode_key(vm: &mut Vm, value: JsValue) -> std::result::Result<String, JsError> {
    let value = key(vm, value, 0, &mut 0, &mut 0)?;
    serde_json::to_string(&value).map_err(|e| error(vm, ApiError::new("DataError", e.to_string())))
}
fn decode_key(vm: &mut Vm, value: &str) -> std::result::Result<JsValue, JsError> {
    serde_json::from_str::<Key>(value)
        .map_err(|e| error(vm, ApiError::new("DataError", e.to_string())))?
        .js(vm)
}
fn path_get(vm: &mut Vm, mut value: JsValue, path: &str) -> std::result::Result<JsValue, JsError> {
    if path.is_empty() {
        return Ok(value);
    }
    for part in path.split('.') {
        if value.object().is_none() {
            return Ok(JsValue::Undefined);
        }
        value = vm.get(value, part)?;
    }
    Ok(value)
}
fn path_set(
    vm: &mut Vm,
    mut value: JsValue,
    path: &str,
    key: JsValue,
) -> std::result::Result<(), JsError> {
    let parts = path.split('.').collect::<Vec<_>>();
    for (i, part) in parts.iter().enumerate() {
        let id = value
            .object()
            .ok_or_else(|| error(vm, ApiError::new("DataError", "key path target")))?;
        if i + 1 == parts.len() {
            vm.define(id, (*part).into(), Property::data(key))?;
            return Ok(());
        }
        let child = vm.get(value.clone(), part)?;
        value = if child == JsValue::Undefined {
            let child = vm.object()?;
            vm.define(id, (*part).into(), Property::data(child.clone()))?;
            child
        } else {
            child
        };
    }
    Ok(())
}
fn valid_path(path: &str) -> bool {
    path.len() <= 1024
        && (path.is_empty()
            || path.split('.').all(|s| {
                !s.is_empty()
                    && s.chars().enumerate().all(|(i, c)| {
                        c == '_' || c == '$' || c.is_alphabetic() || i > 0 && c.is_ascii_digit()
                    })
            }))
}
impl WebHost {
    fn idb(&self, id: u64) -> Option<Idb> {
        match self.state.borrow().objects.get(&id).map(|o| &o.kind) {
            Some(Kind::Idb(value)) => Some(value.clone()),
            _ => None,
        }
    }
    fn list(&self, vm: &mut Vm, values: Vec<String>) -> std::result::Result<JsValue, JsError> {
        let list = vm.array(values.into_iter().map(JsValue::string).collect())?;
        let (contains, item) = {
            let s = self.state.borrow();
            (s.function("db:listContains"), s.function("db:listItem"))
        };
        vm.set(list.clone(), "contains", contains)?;
        vm.set(list.clone(), "item", item)?;
        Ok(list)
    }
    pub(super) fn database_get(
        &self,
        vm: &mut Vm,
        id: u64,
        name: &str,
    ) -> std::result::Result<Option<JsValue>, JsError> {
        let function = |name: &str| Some(self.state.borrow().function(name));
        if id == INDEXED_DB {
            return Ok(match name {
                "open" => function("db:open"),
                "deleteDatabase" => function("db:deleteDatabase"),
                _ => None,
            });
        }
        let value = match self.idb(id) {
            Some(Idb::Connection {
                name: db,
                version,
                closed: _,
                upgrade,
            }) => match name {
                "name" => Some(JsValue::string(&db)),
                "version" => Some(JsValue::Number(version as f64)),
                "objectStoreNames" => {
                    let data =
                        if let Some(Idb::Transaction(t)) = upgrade.and_then(|id| self.idb(id)) {
                            (*t.data).clone()
                        } else {
                            let s = self.state.borrow();
                            s.config
                                .profile
                                .database(&s.dom.url(), &db)
                                .map_err(|e| error(vm, e))?
                        };
                    Some(self.list(vm, data.stores.keys().cloned().collect())?)
                }
                "createObjectStore" => function("db:createObjectStore"),
                "deleteObjectStore" => function("db:deleteObjectStore"),
                "transaction" => function("db:transaction"),
                "close" => function("db:close"),
                _ => None,
            },
            Some(Idb::Transaction(t)) => match name {
                "db" => Some(wrap(vm, t.connection, "IDBDatabase")?),
                "mode" => Some(JsValue::string(t.mode)),
                "error" => self
                    .state
                    .borrow()
                    .objects
                    .get(&id)
                    .and_then(|o| o.properties.get("__error").cloned())
                    .or(Some(JsValue::Null)),
                "objectStoreNames" => Some(self.list(vm, t.scope)?),
                "objectStore" => function("db:objectStore"),
                "abort" => function("db:abort"),
                "commit" => function("db:commit"),
                _ => None,
            },
            Some(Idb::Store {
                transaction,
                name: store,
            }) => {
                let Some(Idb::Transaction(t)) = self.idb(transaction) else {
                    return Ok(None);
                };
                let data = t.data.stores.get(&store);
                match name {
                    "name" => Some(JsValue::string(store)),
                    "keyPath" => Some(
                        data.and_then(|s| s.key_path.as_ref())
                            .map(JsValue::string)
                            .unwrap_or(JsValue::Null),
                    ),
                    "autoIncrement" => Some(JsValue::Bool(data.is_some_and(|s| s.auto_increment))),
                    "transaction" => Some(wrap(vm, transaction, "IDBTransaction")?),
                    "indexNames" => Some(self.list(
                        vm,
                        data.map_or_else(Vec::new, |s| s.indexes.keys().cloned().collect()),
                    )?),
                    "createIndex" => function("db:createIndex"),
                    "deleteIndex" => function("db:deleteIndex"),
                    "index" => function("db:index"),
                    _ => operation_property(name, &function),
                }
            }
            Some(Idb::Index {
                transaction,
                store,
                name: index,
            }) => {
                let data = self.idb(transaction).and_then(|v| match v {
                    Idb::Transaction(t) => t
                        .data
                        .stores
                        .get(&store)
                        .and_then(|s| s.indexes.get(&index))
                        .cloned(),
                    _ => None,
                });
                match name {
                    "name" => Some(JsValue::string(index)),
                    "keyPath" => Some(JsValue::string(data.as_ref().map_or("", |i| &i.key_path))),
                    "unique" => Some(JsValue::Bool(data.is_some_and(|i| i.unique))),
                    "objectStore" => Some(self.allocate(
                        vm,
                        Kind::Idb(Idb::Store {
                            transaction,
                            name: store,
                        }),
                        "IDBObjectStore",
                    )?),
                    _ => operation_property(name, &function),
                }
            }
            Some(Idb::Request {
                result,
                error: e,
                done,
                source,
                transaction,
            }) => match name {
                "result" if done || result != JsValue::Undefined => Some(result),
                "result" => {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "request pending"),
                    ))
                }
                "error" => Some(e),
                "readyState" => Some(JsValue::string(if done { "done" } else { "pending" })),
                "source" => Some(
                    source
                        .map(|id| wrap(vm, id, "IDBObjectStore"))
                        .transpose()?
                        .unwrap_or(JsValue::Null),
                ),
                "transaction" => Some(
                    transaction
                        .map(|id| wrap(vm, id, "IDBTransaction"))
                        .transpose()?
                        .unwrap_or(JsValue::Null),
                ),
                _ => None,
            },
            _ => None,
        };
        Ok(value)
    }
    fn request(
        &self,
        vm: &mut Vm,
        source: Option<u64>,
        transaction: Option<u64>,
    ) -> std::result::Result<(u64, JsValue), JsError> {
        let mut s = self.state.borrow_mut();
        if s.pending.len() >= 64 {
            return Err(error(
                vm,
                ApiError::new("QuotaExceededError", "database pending requests"),
            ));
        }
        let id = s
            .allocate(Kind::Idb(Idb::Request {
                result: JsValue::Undefined,
                error: JsValue::Null,
                done: false,
                source,
                transaction,
            }))
            .map_err(|e| error(vm, e))?;
        s.pending.insert(id);
        drop(s);
        Ok((id, wrap(vm, id, "IDBRequest")?))
    }
    pub(super) fn database_call(
        &self,
        vm: &mut Vm,
        method: &str,
        id: u64,
        args: &[JsValue],
    ) -> std::result::Result<Option<JsValue>, JsError> {
        if !method.starts_with("db:") {
            return Ok(None);
        }
        let undefined = JsValue::Undefined;
        let value = match method {
            "db:open" | "db:deleteDatabase" => {
                if id != INDEXED_DB && id != WINDOW {
                    return Err(error(vm, ApiError::new("TypeError", "IDBFactory receiver")));
                }
                let name = text(vm, args, 0)?;
                if name.len() > 1024 {
                    return Err(error(
                        vm,
                        ApiError::new("QuotaExceededError", "database name"),
                    ));
                }
                let version = if arg(args, 1) == undefined {
                    None
                } else {
                    let n = vm.to_number(arg(args, 1))?;
                    if !n.is_finite() || n < 1.0 || n.fract() != 0.0 || n > 9_007_199_254_740_991.0
                    {
                        return Err(error(vm, ApiError::new("TypeError", "database version")));
                    }
                    Some(n as u64)
                };
                let (request, value) = self.request(vm, None, None)?;
                self.state
                    .borrow_mut()
                    .jobs
                    .push_back(Job::Database(Task::Open {
                        request,
                        name,
                        version,
                        delete: method == "db:deleteDatabase",
                    }));
                value
            }
            "db:close" => {
                let mut s = self.state.borrow_mut();
                if let Some(Object {
                    kind: Kind::Idb(Idb::Connection { closed, .. }),
                    ..
                }) = s.objects.get_mut(&id)
                {
                    *closed = true;
                }
                let tasks = std::mem::take(&mut s.blocked_databases);
                for (_, task) in tasks {
                    s.jobs.push_back(Job::Database(task));
                }
                undefined
            }
            "db:transaction" => {
                let Some(Idb::Connection {
                    name,
                    closed: false,
                    upgrade: None,
                    ..
                }) = self.idb(id)
                else {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "database closed or upgrading"),
                    ));
                };
                let input = arg(args, 0);
                let scope = if let JsValue::String(s) = input {
                    vec![s.to_string()]
                } else {
                    let mut out = Vec::new();
                    for v in vm.array_values(input)? {
                        out.push(vm.to_string(v)?);
                    }
                    out
                };
                if scope.is_empty() || scope.len() > 256 {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidAccessError", "transaction scope"),
                    ));
                }
                let mode = if arg(args, 1) == undefined {
                    "readonly".into()
                } else {
                    text(vm, args, 1)?
                };
                if !matches!(mode.as_str(), "readonly" | "readwrite") {
                    return Err(error(vm, ApiError::new("TypeError", "transaction mode")));
                }
                let data = {
                    let s = self.state.borrow();
                    s.config
                        .profile
                        .database(&s.dom.url(), &name)
                        .map_err(|e| error(vm, e))?
                };
                if scope.iter().any(|name| !data.stores.contains_key(name)) {
                    return Err(error(vm, ApiError::new("NotFoundError", "object store")));
                }
                let mut s = self.state.borrow_mut();
                if s.objects.values().filter(|o|matches!(&o.kind,Kind::Idb(Idb::Transaction(t))if matches!(t.phase,Phase::Waiting|Phase::Active))).count()>=16{return Err(error(vm,ApiError::new("QuotaExceededError","active transactions")));}
                let transaction = s
                    .allocate(Kind::Idb(Idb::Transaction(Transaction {
                        connection: id,
                        name,
                        mode,
                        scope,
                        phase: Phase::Waiting,
                        data: Arc::new(data),
                        pending: 0,
                        dirty: false,
                        open: None,
                        committing: false,
                    })))
                    .map_err(|e| error(vm, e))?;
                s.pending.insert(transaction);
                s.jobs.push_back(Job::Database(Task::Start { transaction }));
                drop(s);
                wrap(vm, transaction, "IDBTransaction")?
            }
            "db:createObjectStore" | "db:deleteObjectStore" => {
                let name = text(vm, args, 0)?;
                let Some(Idb::Connection {
                    upgrade: Some(transaction),
                    ..
                }) = self.idb(id)
                else {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidStateError", "schema change outside upgrade"),
                    ));
                };
                let path = get(vm, arg(args, 1), "keyPath")?;
                let path = if matches!(path, JsValue::Null | JsValue::Undefined) {
                    None
                } else {
                    Some(vm.to_string(path)?)
                };
                if path.as_ref().is_some_and(|p| !valid_path(p)) {
                    return Err(error(vm, ApiError::new("SyntaxError", "key path")));
                }
                let auto = get(vm, arg(args, 1), "autoIncrement")?.truthy();
                if auto && path.as_deref() == Some("") {
                    return Err(error(
                        vm,
                        ApiError::new("InvalidAccessError", "empty auto key path"),
                    ));
                }
                self.mutate_transaction(vm, transaction, |t| {
                    let db = Arc::make_mut(&mut t.data);
                    if method == "db:deleteObjectStore" {
                        if db.stores.remove(&name).is_none() {
                            return Err(ApiError::new("NotFoundError", "object store"));
                        }
                    } else {
                        if db.stores.contains_key(&name) {
                            return Err(ApiError::new("ConstraintError", "object store exists"));
                        }
                        if db.stores.len() >= 256 {
                            return Err(ApiError::new("QuotaExceededError", "object stores"));
                        }
                        db.stores.insert(
                            name.clone(),
                            ObjectStore {
                                key_path: path,
                                auto_increment: auto,
                                next_key: 1,
                                ..Default::default()
                            },
                        );
                        t.scope.push(name.clone());
                    }
                    t.dirty = true;
                    Ok(())
                })?;
                if method == "db:deleteObjectStore" {
                    undefined
                } else {
                    self.allocate(
                        vm,
                        Kind::Idb(Idb::Store { transaction, name }),
                        "IDBObjectStore",
                    )?
                }
            }
            "db:objectStore" => {
                let name = text(vm, args, 0)?;
                let Some(Idb::Transaction(t)) = self.idb(id) else {
                    return Err(error(
                        vm,
                        ApiError::new("TypeError", "transaction receiver"),
                    ));
                };
                if !matches!(t.phase, Phase::Waiting | Phase::Active) || !t.scope.contains(&name) {
                    return Err(error(
                        vm,
                        ApiError::new("NotFoundError", "transaction object store"),
                    ));
                }
                self.allocate(
                    vm,
                    Kind::Idb(Idb::Store {
                        transaction: id,
                        name,
                    }),
                    "IDBObjectStore",
                )?
            }
            "db:abort" | "db:commit" => {
                self.mutate_transaction(vm, id, |t| {
                    if matches!(t.phase, Phase::Done | Phase::Aborted) {
                        return Err(ApiError::new("InvalidStateError", "transaction finished"));
                    }
                    if method == "db:abort" {
                        t.phase = Phase::Aborted;
                    } else {
                        t.committing = true;
                    }
                    Ok(())
                })?;
                self.state
                    .borrow_mut()
                    .jobs
                    .push_back(Job::Database(Task::Commit { transaction: id }));
                undefined
            }
            "db:createIndex" | "db:deleteIndex" | "db:index" => {
                let name = text(vm, args, 0)?;
                let Some(Idb::Store {
                    transaction,
                    name: store,
                }) = self.idb(id)
                else {
                    return Err(error(
                        vm,
                        ApiError::new("TypeError", "object store receiver"),
                    ));
                };
                if method == "db:index" {
                    let Some(Idb::Transaction(t)) = self.idb(transaction) else {
                        return Err(error(vm, ApiError::new("InvalidStateError", "transaction")));
                    };
                    if !t
                        .data
                        .stores
                        .get(&store)
                        .is_some_and(|s| s.indexes.contains_key(&name))
                    {
                        return Err(error(vm, ApiError::new("NotFoundError", "index")));
                    }
                } else {
                    let path = if method == "db:createIndex" {
                        text(vm, args, 1)?
                    } else {
                        String::new()
                    };
                    if !valid_path(&path) {
                        return Err(error(vm, ApiError::new("SyntaxError", "index key path")));
                    }
                    let unique = get(vm, arg(args, 2), "unique")?.truthy();
                    if get(vm, arg(args, 2), "multiEntry")?.truthy() {
                        return Err(error(
                            vm,
                            ApiError::new("NotSupportedError", "multiEntry index"),
                        ));
                    }
                    self.mutate_transaction(vm, transaction, |t| {
                        if t.mode != "versionchange" {
                            return Err(ApiError::new(
                                "InvalidStateError",
                                "schema change outside upgrade",
                            ));
                        }
                        let store = Arc::make_mut(&mut t.data)
                            .stores
                            .get_mut(&store)
                            .ok_or_else(|| ApiError::new("NotFoundError", "object store"))?;
                        if method == "db:deleteIndex" {
                            if store.indexes.remove(&name).is_none() {
                                return Err(ApiError::new("NotFoundError", "index"));
                            }
                        } else {
                            if store.indexes.contains_key(&name) {
                                return Err(ApiError::new("ConstraintError", "index exists"));
                            }
                            if store.indexes.len() >= 64 {
                                return Err(ApiError::new("QuotaExceededError", "index count"));
                            }
                            store.indexes.insert(
                                name.clone(),
                                Index {
                                    key_path: path,
                                    unique,
                                    ..Default::default()
                                },
                            );
                        }
                        t.dirty = true;
                        Ok(())
                    })?;
                }
                if method == "db:deleteIndex" {
                    undefined
                } else {
                    self.allocate(
                        vm,
                        Kind::Idb(Idb::Index {
                            transaction,
                            store,
                            name,
                        }),
                        "IDBIndex",
                    )?
                }
            }
            _ => {
                let (transaction, store, index) = match self.idb(id) {
                    Some(Idb::Store { transaction, name }) => (transaction, name, None),
                    Some(Idb::Index {
                        transaction,
                        store,
                        name,
                    }) => (transaction, store, Some(name)),
                    _ => {
                        return Err(error(
                            vm,
                            ApiError::new("TypeError", "database source receiver"),
                        ))
                    }
                };
                let Some(Idb::Transaction(t)) = self.idb(transaction) else {
                    return Err(error(
                        vm,
                        ApiError::new("TransactionInactiveError", "transaction expired"),
                    ));
                };
                if !matches!(t.phase, Phase::Waiting | Phase::Active) || t.committing {
                    return Err(error(
                        vm,
                        ApiError::new("TransactionInactiveError", "transaction finished"),
                    ));
                }
                let write = matches!(method, "db:put" | "db:add" | "db:delete" | "db:clear");
                if write && (t.mode == "readonly" || index.is_some()) {
                    return Err(error(
                        vm,
                        ApiError::new("ReadOnlyError", "write in readonly transaction"),
                    ));
                }
                let value = if matches!(method, "db:put" | "db:add") {
                    Some(vm.serialize_clone(arg(args, 0)).map_err(|e| {
                        if e.is_resource_limit() {
                            e
                        } else {
                            error(vm, ApiError::new("DataCloneError", e.to_string()))
                        }
                    })?)
                } else {
                    None
                };
                let input = if value.is_some() {
                    arg(args, 1)
                } else {
                    arg(args, 0)
                };
                let key = if input == undefined || matches!(method, "db:clear") {
                    None
                } else {
                    Some(encode_key(vm, input)?)
                };
                if key.is_none() && matches!(method, "db:get" | "db:getKey" | "db:delete") {
                    return Err(error(vm, ApiError::new("DataError", "key required")));
                }
                let count = if arg(args, 1) == undefined || value.is_some() {
                    100_000
                } else {
                    vm.to_number(arg(args, 1))?.clamp(0.0, 100_000.0) as usize
                };
                let (request, wrapper) = self.request(vm, Some(id), Some(transaction))?;
                self.mutate_transaction(vm, transaction, |t| {
                    t.pending += 1;
                    Ok(())
                })?;
                self.state
                    .borrow_mut()
                    .jobs
                    .push_back(Job::Database(Task::Operation {
                        request,
                        transaction,
                        store,
                        index,
                        operation: method.into(),
                        key,
                        value,
                        count,
                    }));
                wrapper
            }
        };
        Ok(Some(value))
    }
    fn mutate_transaction(
        &self,
        vm: &mut Vm,
        id: u64,
        operation: impl FnOnce(&mut Transaction) -> Result<()>,
    ) -> std::result::Result<(), JsError> {
        let mut s = self.state.borrow_mut();
        let Some(Object {
            kind: Kind::Idb(Idb::Transaction(t)),
            ..
        }) = s.objects.get_mut(&id)
        else {
            return Err(error(vm, ApiError::new("InvalidStateError", "transaction")));
        };
        operation(t).map_err(|e| error(vm, e))
    }
}
fn operation_property(name: &str, function: &impl Fn(&str) -> Option<JsValue>) -> Option<JsValue> {
    match name {
        "put" => function("db:put"),
        "add" => function("db:add"),
        "get" => function("db:get"),
        "getKey" => function("db:getKey"),
        "getAll" => function("db:getAll"),
        "delete" => function("db:delete"),
        "clear" => function("db:clear"),
        "count" => function("db:count"),
        _ => None,
    }
}
impl PageRuntime {
    pub(super) fn database_task(&mut self, task: Task) -> std::result::Result<(), JsError> {
        let host = WebHost {
            state: self.state.clone(),
        };
        match task {
            Task::Open {
                request,
                name,
                version,
                delete,
            } => {
                let (url, profile) = {
                    let s = self.state.borrow();
                    (s.dom.url(), s.config.profile.clone())
                };
                let data = profile
                    .database(&url, &name)
                    .map_err(|e| error(&mut self.dom.vm, e))?;
                let version = version.unwrap_or(data.version.max(1));
                if version < data.version && !delete {
                    return self.database_request(
                        request,
                        Err(ApiError::new("VersionError", "lower database version")),
                        None,
                    );
                }
                if version > data.version || delete {
                    let connections=self.state.borrow().objects.iter().filter_map(|(id,o)|matches!(&o.kind,Kind::Idb(Idb::Connection{name:n,closed:false,..})if n==&name).then_some(*id)).collect::<Vec<_>>();
                    for id in &connections {
                        let detail = self.version_detail(
                            data.version,
                            if delete { None } else { Some(version) },
                        )?;
                        self.deliver_event(*id, "versionchange", detail)?;
                    }
                    if connections.iter().any(|id| {
                        matches!(host.idb(*id), Some(Idb::Connection { closed: false, .. }))
                    }) {
                        self.state.borrow_mut().blocked_databases.insert(
                            request,
                            Task::Open {
                                request,
                                name,
                                version: Some(version),
                                delete,
                            },
                        );
                        let detail = self.version_detail(data.version, Some(version))?;
                        self.deliver_event(request, "blocked", detail)?;
                        return Ok(());
                    }
                }
                if delete {
                    let result = profile
                        .delete_database(&url, &name)
                        .map(|_| JsValue::Undefined);
                    return self.database_request(request, result, None);
                }
                let connection = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::Idb(Idb::Connection {
                        name: name.clone(),
                        version,
                        closed: false,
                        upgrade: None,
                    }))
                    .map_err(|e| error(&mut self.dom.vm, e))?;
                let value = wrap(&mut self.dom.vm, connection, "IDBDatabase")?;
                if version == data.version {
                    return self.database_request(request, Ok(value), None);
                }
                let mut updated = data.clone();
                updated.version = version;
                let transaction = self
                    .state
                    .borrow_mut()
                    .allocate(Kind::Idb(Idb::Transaction(Transaction {
                        connection,
                        name,
                        mode: "versionchange".into(),
                        scope: updated.stores.keys().cloned().collect(),
                        phase: Phase::Active,
                        data: Arc::new(updated),
                        pending: 0,
                        dirty: true,
                        open: Some(request),
                        committing: false,
                    })))
                    .map_err(|e| error(&mut self.dom.vm, e))?;
                {
                    let mut s = self.state.borrow_mut();
                    s.pending.insert(transaction);
                    if let Some(Object {
                        kind: Kind::Idb(Idb::Connection { upgrade, .. }),
                        ..
                    }) = s.objects.get_mut(&connection)
                    {
                        *upgrade = Some(transaction);
                    }
                    if let Some(Object {
                        kind:
                            Kind::Idb(Idb::Request {
                                result,
                                transaction: t,
                                ..
                            }),
                        ..
                    }) = s.objects.get_mut(&request)
                    {
                        *result = value;
                        *t = Some(transaction);
                    }
                }
                let detail = self.version_detail(data.version, Some(version))?;
                let (succeeded, _) = self.web_event(request, "upgradeneeded", detail)?;
                if !succeeded {
                    host.mutate_transaction(&mut self.dom.vm, transaction, |t| {
                        t.phase = Phase::Aborted;
                        Ok(())
                    })?;
                }
                self.state
                    .borrow_mut()
                    .jobs
                    .push_back(Job::Database(Task::Commit { transaction }));
            }
            Task::Start { transaction } => {
                let Some(Idb::Transaction(mut t)) = host.idb(transaction) else {
                    return Ok(());
                };
                if t.phase != Phase::Waiting {
                    return Ok(());
                }
                let conflict=self.state.borrow().objects.iter().any(|(id,o)|*id!=transaction&&matches!(&o.kind,Kind::Idb(Idb::Transaction(other))if other.phase==Phase::Active&&other.name==t.name&&(other.mode!="readonly"||t.mode!="readonly")&&other.scope.iter().any(|name|t.scope.contains(name))));
                if conflict {
                    self.state
                        .borrow_mut()
                        .jobs
                        .push_back(Job::Database(Task::Start { transaction }));
                    return Ok(());
                }
                let (url, profile) = {
                    let s = self.state.borrow();
                    (s.dom.url(), s.config.profile.clone())
                };
                t.data = Arc::new(
                    profile
                        .database(&url, &t.name)
                        .map_err(|e| error(&mut self.dom.vm, e))?,
                );
                t.phase = Phase::Active;
                if let Some(object) = self.state.borrow_mut().objects.get_mut(&transaction) {
                    object.kind = Kind::Idb(Idb::Transaction(t));
                }
                self.state
                    .borrow_mut()
                    .jobs
                    .push_back(Job::Database(Task::Commit { transaction }));
            }
            Task::Commit { transaction } => {
                let Some(Idb::Transaction(mut t)) = host.idb(transaction) else {
                    return Ok(());
                };
                if t.phase == Phase::Done {
                    return Ok(());
                }
                if t.pending > 0 || t.phase == Phase::Waiting {
                    self.state
                        .borrow_mut()
                        .jobs
                        .push_back(Job::Database(Task::Commit { transaction }));
                    return Ok(());
                }
                let result = if t.phase == Phase::Aborted {
                    Err(ApiError::new("AbortError", "transaction aborted"))
                } else if t.dirty {
                    let result = rebuild_indexes(&mut self.dom.vm, Arc::make_mut(&mut t.data));
                    if let Err(e) = result {
                        if e.is_resource_limit() {
                            return Err(e);
                        }
                        Err(ApiError::new("ConstraintError", e.message))
                    } else {
                        let (url, profile) = {
                            let s = self.state.borrow();
                            (s.dom.url(), s.config.profile.clone())
                        };
                        profile.save_database(&url, &t.name, (*t.data).clone())
                    }
                } else {
                    Ok(())
                };
                let ok = result.is_ok();
                t.phase = Phase::Done;
                {
                    let mut s = self.state.borrow_mut();
                    s.pending.remove(&transaction);
                    if let Some(object) = s.objects.get_mut(&transaction) {
                        object.kind = Kind::Idb(Idb::Transaction(t.clone()));
                    }
                    if let Some(Object {
                        kind:
                            Kind::Idb(Idb::Connection {
                                upgrade, closed, ..
                            }),
                        ..
                    }) = s.objects.get_mut(&t.connection)
                    {
                        *upgrade = None;
                        if !ok && t.open.is_some() {
                            *closed = true;
                        }
                    }
                }
                if let Err(e) = &result {
                    let value = error(&mut self.dom.vm, e.clone())
                        .thrown
                        .unwrap_or(JsValue::Null);
                    if let Some(object) = self.state.borrow_mut().objects.get_mut(&transaction) {
                        object.properties.insert("__error".into(), value);
                    }
                }
                self.deliver_event(
                    transaction,
                    if ok { "complete" } else { "abort" },
                    JsValue::Undefined,
                )?;
                if let Some(request) = t.open {
                    let value = wrap(&mut self.dom.vm, t.connection, "IDBDatabase")?;
                    self.database_request(request, result.map(|_| value), None)?;
                }
            }
            Task::Operation {
                request,
                transaction,
                store,
                index,
                operation,
                key,
                value,
                count,
            } => {
                let Some(Idb::Transaction(mut t)) = host.idb(transaction) else {
                    return Ok(());
                };
                if t.phase == Phase::Waiting {
                    self.state
                        .borrow_mut()
                        .jobs
                        .push_back(Job::Database(Task::Operation {
                            request,
                            transaction,
                            store,
                            index,
                            operation,
                            key,
                            value,
                            count,
                        }));
                    return Ok(());
                }
                let result = if t.phase == Phase::Aborted {
                    Err(error(
                        &mut self.dom.vm,
                        ApiError::new("AbortError", "transaction aborted"),
                    ))
                } else {
                    operate(
                        &mut self.dom.vm,
                        &mut t,
                        &store,
                        index.as_deref(),
                        &operation,
                        key,
                        value,
                        count,
                    )
                };
                t.pending = t.pending.saturating_sub(1);
                if let Some(object) = self.state.borrow_mut().objects.get_mut(&transaction) {
                    object.kind = Kind::Idb(Idb::Transaction(t));
                }
                let result = match result {
                    Ok(value) => Ok(value),
                    Err(e) if e.is_resource_limit() => return Err(e),
                    Err(e) => Err(ApiError::new(
                        e.thrown
                            .as_ref()
                            .and_then(|v| self.dom.vm.get(v.clone(), "name").ok())
                            .as_ref()
                            .and_then(|v| match v {
                                JsValue::String(s) => Some(s.as_ref()),
                                _ => None,
                            })
                            .unwrap_or("DataError"),
                        e.message,
                    )),
                };
                self.database_request(request, result, Some(transaction))?;
            }
        }
        Ok(())
    }
    fn version_detail(
        &mut self,
        old: u64,
        new: Option<u64>,
    ) -> std::result::Result<JsValue, JsError> {
        let detail = self.dom.vm.object()?;
        self.dom
            .vm
            .set(detail.clone(), "oldVersion", JsValue::Number(old as f64))?;
        self.dom.vm.set(
            detail.clone(),
            "newVersion",
            new.map_or(JsValue::Null, |v| JsValue::Number(v as f64)),
        )?;
        Ok(detail)
    }
    fn database_request(
        &mut self,
        id: u64,
        result: Result<JsValue>,
        transaction: Option<u64>,
    ) -> std::result::Result<(), JsError> {
        let ok = result.is_ok();
        let (value, e) = match result {
            Ok(value) => (value, JsValue::Null),
            Err(e) => (
                JsValue::Undefined,
                error(&mut self.dom.vm, e).thrown.unwrap_or(JsValue::Null),
            ),
        };
        {
            let mut s = self.state.borrow_mut();
            s.pending.remove(&id);
            if let Some(Object {
                kind:
                    Kind::Idb(Idb::Request {
                        result,
                        error,
                        done,
                        transaction: t,
                        ..
                    }),
                ..
            }) = s.objects.get_mut(&id)
            {
                *result = value;
                *error = e;
                *done = true;
                if transaction.is_none() {
                    *t = None;
                }
            }
        }
        let (succeeded, prevented) =
            self.web_event(id, if ok { "success" } else { "error" }, JsValue::Undefined)?;
        if (!ok && !prevented) || !succeeded {
            if let Some(transaction) = transaction {
                WebHost {
                    state: self.state.clone(),
                }
                .mutate_transaction(&mut self.dom.vm, transaction, |t| {
                    t.phase = Phase::Aborted;
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn operate(
    vm: &mut Vm,
    t: &mut Transaction,
    store: &str,
    index: Option<&str>,
    operation: &str,
    input_key: Option<String>,
    input: Option<CloneData>,
    count: usize,
) -> std::result::Result<JsValue, JsError> {
    let mut data = t
        .data
        .stores
        .get(store)
        .cloned()
        .ok_or_else(|| error(vm, ApiError::new("NotFoundError", "object store")))?;
    let output = match operation {
        "db:put" | "db:add" => {
            let input = input.ok_or_else(|| error(vm, ApiError::new("DataCloneError", "value")))?;
            let mut value = vm.deserialize_clone(&input)?;
            if data.key_path.is_some() && input_key.is_some() {
                return Err(error(
                    vm,
                    ApiError::new("DataError", "explicit key with inline key path"),
                ));
            }
            let mut primary = if let Some(path) = &data.key_path {
                let v = path_get(vm, value.clone(), path)?;
                if v == JsValue::Undefined {
                    None
                } else {
                    Some(encode_key(vm, v)?)
                }
            } else {
                input_key
            };
            if primary.is_none() {
                if !data.auto_increment {
                    return Err(error(vm, ApiError::new("DataError", "key required")));
                }
                let n = data.next_key.max(1);
                if n > 9_007_199_254_740_991 {
                    return Err(error(
                        vm,
                        ApiError::new("ConstraintError", "key generator exhausted"),
                    ));
                }
                let key = JsValue::Number(n as f64);
                primary = Some(encode_key(vm, key.clone())?);
                data.next_key = n + 1;
                if let Some(path) = &data.key_path {
                    path_set(vm, value.clone(), path, key)?;
                }
            }
            let primary = primary.unwrap_or_default();
            if operation == "db:add" && data.records.contains_key(&primary) {
                return Err(error(vm, ApiError::new("ConstraintError", "key exists")));
            }
            if let Ok(Key::Number(n)) = serde_json::from_str(&primary) {
                if n >= data.next_key as f64 {
                    data.next_key = (n.floor() as u64).saturating_add(1);
                }
            }
            let clone = vm.serialize_clone(std::mem::replace(&mut value, JsValue::Undefined))?;
            data.records.insert(
                primary.clone(),
                serde_json::to_value(clone)
                    .map_err(|e| error(vm, ApiError::new("DataCloneError", e.to_string())))?,
            );
            if data.records.len() > 100_000 {
                return Err(error(
                    vm,
                    ApiError::new("QuotaExceededError", "record count"),
                ));
            }
            rebuild_store(vm, &mut data)?;
            Arc::make_mut(&mut t.data).stores.insert(store.into(), data);
            t.dirty = true;
            return decode_key(vm, &primary);
        }
        "db:delete" => {
            if let Some(key) = input_key {
                data.records.remove(&key);
            }
            JsValue::Undefined
        }
        "db:clear" => {
            data.records.clear();
            JsValue::Undefined
        }
        _ => {
            // Key ordering is IndexedDB ordering, independent of JSON/map ordering.
            let keys = if let Some(index) = index {
                rebuild_store(vm, &mut data)?;
                let index = data
                    .indexes
                    .get(index)
                    .ok_or_else(|| error(vm, ApiError::new("NotFoundError", "index")))?;
                if let Some(key) = input_key {
                    index.values.get(&key).cloned().unwrap_or_default()
                } else {
                    let mut entries = index.values.iter().collect::<Vec<_>>();
                    entries.sort_by(|(a, _), (b, _)| compare_encoded(a, b));
                    entries
                        .into_iter()
                        .flat_map(|(_, v)| v.iter().cloned())
                        .collect()
                }
            } else {
                let mut keys = data
                    .records
                    .keys()
                    .filter(|key| input_key.as_ref().is_none_or(|input| *key == input))
                    .cloned()
                    .collect::<Vec<_>>();
                keys.sort_by(|a, b| compare_encoded(a, b));
                keys
            };
            if operation == "db:count" {
                return Ok(JsValue::Number(keys.len() as f64));
            }
            if operation == "db:getKey" {
                return keys
                    .first()
                    .map(|k| decode_key(vm, k))
                    .unwrap_or(Ok(JsValue::Undefined));
            }
            let mut values = Vec::new();
            for key in keys
                .into_iter()
                .take(if operation == "db:getAll" { count } else { 1 })
            {
                vm.checkpoint()?;
                if let Some(value) = data.records.get(&key) {
                    let clone: CloneData = serde_json::from_value(value.clone())
                        .map_err(|e| error(vm, ApiError::new("DataCloneError", e.to_string())))?;
                    values.push(vm.deserialize_clone(&clone)?);
                }
            }
            return if operation == "db:getAll" {
                vm.array(values)
            } else {
                Ok(values.into_iter().next().unwrap_or(JsValue::Undefined))
            };
        }
    };
    rebuild_store(vm, &mut data)?;
    Arc::make_mut(&mut t.data).stores.insert(store.into(), data);
    t.dirty = true;
    Ok(output)
}
fn compare_encoded(a: &str, b: &str) -> std::cmp::Ordering {
    match (
        serde_json::from_str::<Key>(a),
        serde_json::from_str::<Key>(b),
    ) {
        (Ok(a), Ok(b)) => a.compare(&b),
        _ => a.cmp(b),
    }
}
fn rebuild_indexes(vm: &mut Vm, db: &mut Database) -> std::result::Result<(), JsError> {
    for store in db.stores.values_mut() {
        rebuild_store(vm, store)?;
    }
    Ok(())
}
fn rebuild_store(vm: &mut Vm, store: &mut ObjectStore) -> std::result::Result<(), JsError> {
    if store.indexes.is_empty() {
        return Ok(());
    }
    for index in store.indexes.values_mut() {
        index.values.clear();
    }
    for (primary, record) in &store.records {
        vm.checkpoint()?;
        let data: CloneData = serde_json::from_value(record.clone())
            .map_err(|e| error(vm, ApiError::new("DataCloneError", e.to_string())))?;
        let value = vm.deserialize_clone(&data)?;
        for index in store.indexes.values_mut() {
            let key = path_get(vm, value.clone(), &index.key_path)?;
            let encoded = match encode_key(vm, key) {
                Ok(key) => key,
                Err(e) if e.is_resource_limit() => return Err(e),
                Err(_) => continue,
            };
            let entries = index.values.entry(encoded).or_default();
            if index.unique && !entries.is_empty() {
                return Err(error(vm, ApiError::new("ConstraintError", "unique index")));
            }
            entries.push(primary.clone());
        }
    }
    for index in store.indexes.values_mut() {
        for values in index.values.values_mut() {
            values.sort_by(|a, b| compare_encoded(a, b));
        }
    }
    Ok(())
}
