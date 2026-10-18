//! Source-text module records, live import aliases, namespace objects and async
//! evaluation. Loading is a host message; this layer never performs I/O.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Unlinked,
    Linking,
    Linked,
    Evaluating,
    Evaluated,
    Errored,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportBinding {
    Binding { environment: usize, name: String },
    Namespace(usize),
}
#[derive(Debug, Clone)]
pub(crate) struct Module {
    meta: Option<JsValue>,
    link_error: Option<JsError>,
    pub url: String,
    pub environment: usize,
    program: Program,
    code: Arc<crate::bytecode::Chunk>,
    dependencies: Vec<(String, usize)>,
    status: Status,
    namespace: Option<JsValue>,
    promise: Option<JsValue>,
    error: Option<JsValue>,
    remaining: usize,
    pub bytes: usize,
}
pub fn resolve_module_url(base: &str, specifier: &str) -> Result<String, JsError> {
    if specifier.len() > 16384 {
        return Err(JsError::new(
            ErrorKind::TypeError,
            "module URL length quota",
        ));
    }
    if !specifier.starts_with(['.', '/']) && url::Url::parse(specifier).is_err() {
        return Err(JsError::new(
            ErrorKind::TypeError,
            "unmapped bare module specifier",
        ));
    }
    url::Url::parse(base)
        .and_then(|base| base.join(specifier))
        .map(|url| url.to_string())
        .map_err(|e| JsError::new(ErrorKind::TypeError, e.to_string()))
}
impl Vm {
    pub fn has_module(&self, url: &str) -> bool {
        self.module_urls.contains_key(url)
    }
    pub fn loaded_module_urls(&self) -> Vec<String> {
        self.module_urls.keys().cloned().collect()
    }
    pub fn alias_module(&mut self, alias: &str, canonical: &str) -> Result<(), JsError> {
        if self.module_urls.contains_key(alias) {
            return Ok(());
        }
        if self.module_urls.len() >= 512 || alias.len() > 16384 {
            return Err(JsError::new(ErrorKind::MemoryLimit, "module alias quota"));
        }
        let id = self
            .module_urls
            .get(canonical)
            .copied()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "canonical module not loaded"))?;
        self.reserve_memory(alias.len() + 64)?;
        self.module_urls.insert(alias.into(), id);
        Ok(())
    }
    pub fn set_module_base(&mut self, key: &str, base: &str) {
        if let Some(&id) = self.module_urls.get(key) {
            self.modules[id].url = base.into();
        }
    }
    pub fn set_source_url(&mut self, url: &str) {
        self.source_url = url.to_owned();
    }
    pub fn source_url(&self) -> &str {
        &self.source_url
    }
    pub fn define_module(&mut self, url: &str, source: &str) -> Result<Vec<String>, JsError> {
        if let Some(&index) = self.module_urls.get(url) {
            return Ok(self.modules[index].program.requests.clone());
        }
        if self.modules.len() >= 128 {
            return Err(JsError::new(ErrorKind::MemoryLimit, "module record quota"));
        }
        let program = crate::parse_module(source, ParseOpts::default()).map_err(|e| JsError {
            kind: ErrorKind::SyntaxError,
            message: e.message,
            line: e.line,
            column: e.column,
            stack: vec![url.into()],
            thrown: None,
        })?;
        let code = crate::bytecode::compile(&program)?;
        let bytes = source
            .len()
            .saturating_mul(8)
            .saturating_add(code.retained_bytes())
            .saturating_add(1024);
        self.reserve_memory(bytes)?;
        let environment = self.env(0, true, JsValue::Undefined)?;
        let requests = program.requests.clone();
        let index = self.modules.len();
        self.modules.push(Module {
            meta: None,
            link_error: None,
            url: url.into(),
            environment,
            program,
            code,
            dependencies: Vec::new(),
            status: Status::Unlinked,
            namespace: None,
            promise: None,
            error: None,
            remaining: 0,
            bytes,
        });
        self.module_urls.insert(url.into(), index);
        Ok(requests)
    }
    fn dependency(&self, module: usize, source: &str) -> Result<usize, JsError> {
        self.modules[module]
            .dependencies
            .iter()
            .find(|(s, _)| s == source)
            .map(|(_, id)| *id)
            .ok_or_else(|| JsError::new(ErrorKind::SyntaxError, "module dependency is not linked"))
    }
    fn resolve_export(
        &self,
        module: usize,
        name: &str,
        visiting: &mut HashSet<(usize, String)>,
    ) -> Result<Option<ExportBinding>, JsError> {
        if !visiting.insert((module, name.into())) {
            return Ok(None);
        }
        let record = &self.modules[module];
        for export in &record.program.exports {
            match export {
                ExportEntry::Local { exported, local } if exported == name => {
                    if let Some(import) = record.program.imports.iter().find(|i| i.local == *local)
                    {
                        let dep = self.dependency(module, &import.source)?;
                        return if import.namespace {
                            Ok(Some(ExportBinding::Namespace(dep)))
                        } else {
                            self.resolve_export(dep, &import.imported, visiting)
                        };
                    }
                    if !self.scope(record.environment)?.bindings.contains_key(local) {
                        return Err(JsError::new(
                            ErrorKind::SyntaxError,
                            "export has no local binding",
                        ));
                    }
                    return Ok(Some(ExportBinding::Binding {
                        environment: record.environment,
                        name: local.clone(),
                    }));
                }
                ExportEntry::Indirect {
                    exported,
                    source,
                    imported,
                } if exported == name => {
                    let dep = self.dependency(module, source)?;
                    return if imported == "*namespace*" {
                        Ok(Some(ExportBinding::Namespace(dep)))
                    } else {
                        self.resolve_export(dep, imported, visiting)
                    };
                }
                _ => {}
            }
        }
        if name == "default" {
            return Ok(None);
        }
        let mut resolved = None;
        for export in &record.program.exports {
            if let ExportEntry::Star { source } = export {
                if let Some(binding) = self.resolve_export(
                    self.dependency(module, source)?,
                    name,
                    &mut visiting.clone(),
                )? {
                    if resolved.as_ref().is_some_and(|old| *old != binding) {
                        return Ok(None);
                    }
                    resolved = Some(binding);
                }
            }
        }
        Ok(resolved)
    }
    pub(crate) fn module_names(&self, module: usize) -> Result<Vec<String>, JsError> {
        fn names(
            vm: &Vm,
            module: usize,
            seen: &mut HashSet<usize>,
            out: &mut BTreeSet<String>,
        ) -> Result<(), JsError> {
            if !seen.insert(module) {
                return Ok(());
            }
            for export in &vm.modules[module].program.exports {
                match export {
                    ExportEntry::Local { exported, .. }
                    | ExportEntry::Indirect { exported, .. } => {
                        out.insert(exported.clone());
                    }
                    ExportEntry::Star { source } => {
                        let mut star = BTreeSet::new();
                        names(vm, vm.dependency(module, source)?, seen, &mut star)?;
                        star.remove("default");
                        out.extend(star);
                    }
                }
            }
            Ok(())
        }
        let mut all = BTreeSet::new();
        names(self, module, &mut HashSet::new(), &mut all)?;
        let mut out = Vec::new();
        for name in all {
            if self
                .resolve_export(module, &name, &mut HashSet::new())?
                .is_some()
            {
                out.push(name);
            }
        }
        Ok(out)
    }
    pub(crate) fn module_get(
        &mut self,
        module: usize,
        key: &PropertyKey,
    ) -> Result<JsValue, JsError> {
        let PropertyKey::String(name) = key else {
            return Ok(JsValue::Undefined);
        };
        let Some(binding) = self.resolve_export(module, name, &mut HashSet::new())? else {
            return Ok(JsValue::Undefined);
        };
        match binding {
            ExportBinding::Binding { environment, name } => self.lookup(environment, &name),
            ExportBinding::Namespace(module) => self.module_namespace(module),
        }
    }
    fn module_namespace(&mut self, module: usize) -> Result<JsValue, JsError> {
        if let Some(namespace) = &self.modules[module].namespace {
            return Ok(namespace.clone());
        }
        let id = self.allocate(ObjectKind::ModuleNamespace(module), None)?;
        self.store_property(
            id,
            PropertyKey::Symbol(crate::symbols::TO_STRING_TAG),
            Property {
                writable: false,
                enumerable: false,
                configurable: false,
                ..Property::data(JsValue::string("Module"))
            },
        )?;
        self.heap.get_mut(id)?.extensible = false;
        let value = JsValue::Object(id);
        self.modules[module].namespace = Some(value.clone());
        Ok(value)
    }
    fn link_module(&mut self, module: usize) -> Result<(), JsError> {
        if let Some(error) = &self.modules[module].link_error {
            return Err(error.clone());
        }
        let result = self.link_module_inner(module);
        if let Err(error) = &result {
            for record in &mut self.modules {
                if record.status == Status::Linking {
                    record.status = Status::Errored;
                    record.link_error = Some(error.clone());
                }
            }
        }
        result
    }
    fn link_module_inner(&mut self, module: usize) -> Result<(), JsError> {
        if self.modules[module].status != Status::Unlinked {
            return Ok(());
        }
        self.modules[module].status = Status::Linking;
        let record = self.modules[module].clone();
        let mut deps = Vec::new();
        for source in &record.program.requests {
            let url = resolve_module_url(&record.url, source)?;
            let id = self.module_urls.get(&url).copied().ok_or_else(|| {
                JsError::new(
                    ErrorKind::TypeError,
                    format!("module source not loaded: {url}"),
                )
            })?;
            if !deps.iter().any(|(s, _)| s == source) {
                deps.push((source.clone(), id));
            }
        }
        self.modules[module].dependencies = deps.clone();
        let strict = self.strict;
        let url = self.source_url.clone();
        let current = self.current_module.replace(module);
        self.strict = true;
        self.source_url = record.url.clone();
        let result = self.enter_plan(&record.code.plan, record.environment);
        self.strict = strict;
        self.source_url = url;
        self.current_module = current;
        result?;
        for (_, id) in deps {
            self.link_module(id)?;
        }
        for import in record.program.imports {
            if self
                .scope(record.environment)?
                .bindings
                .contains_key(&import.local)
                || self
                    .scope(record.environment)?
                    .aliases
                    .contains_key(&import.local)
            {
                return Err(JsError::new(
                    ErrorKind::SyntaxError,
                    "duplicate import binding",
                ));
            }
            let dep = self.dependency(module, &import.source)?;
            let binding = if import.namespace {
                ExportBinding::Namespace(dep)
            } else {
                self.resolve_export(dep, &import.imported, &mut HashSet::new())?
                    .ok_or_else(|| {
                        JsError::new(
                            ErrorKind::SyntaxError,
                            format!("missing or ambiguous export {}", import.imported),
                        )
                    })?
            };
            match binding {
                ExportBinding::Binding { environment, name } => {
                    self.reserve_memory(import.local.len() + name.len() + 128)?;
                    if let Some(scope) = self.environments[record.environment].as_mut() {
                        scope.aliases.insert(import.local, (environment, name));
                    }
                }
                ExportBinding::Namespace(id) => {
                    let value = self.module_namespace(id)?;
                    self.declare_name(
                        record.environment,
                        &import.local,
                        DeclKind::Const,
                        value,
                        true,
                    )?;
                }
            }
        }
        for export in &record.program.exports {
            if let ExportEntry::Local { exported, .. } | ExportEntry::Indirect { exported, .. } =
                export
            {
                if self
                    .resolve_export(module, exported, &mut HashSet::new())?
                    .is_none()
                {
                    return Err(JsError::new(ErrorKind::SyntaxError, "unresolved export"));
                }
            }
        }
        self.modules[module].status = Status::Linked;
        Ok(())
    }
    fn evaluate_module(
        &mut self,
        module: usize,
        stack: &mut Vec<usize>,
    ) -> Result<JsValue, JsError> {
        if let Some(promise) = &self.modules[module].promise {
            return Ok(promise.clone());
        }
        let promise = self.promise()?;
        self.modules[module].promise = Some(promise.clone());
        self.modules[module].status = Status::Evaluating;
        stack.push(module);
        let deps = self.modules[module].dependencies.clone();
        let mut pending = Vec::new();
        let mut seen = HashSet::new();
        for (_, dep) in deps {
            if !seen.insert(dep) || stack.contains(&dep) {
                continue;
            }
            let dependency = self.evaluate_module(dep, stack)?;
            match self.promise_result(&dependency) {
                Some(Ok(_)) => {}
                Some(Err(reason)) => {
                    stack.pop();
                    self.module_failed(module, reason)?;
                    return Ok(promise);
                }
                None => pending.push(dependency),
            }
        }
        stack.pop();
        self.modules[module].remaining = pending.len();
        if pending.is_empty() {
            self.start_module(module)?;
        } else {
            for promise in pending {
                self.watch_module_dependency(module, promise)?;
            }
        }
        Ok(promise)
    }
    pub(crate) fn module_dependency(
        &mut self,
        module: usize,
        result: Result<JsValue, JsValue>,
    ) -> Result<(), JsError> {
        if self.modules[module].status != Status::Evaluating {
            return Ok(());
        }
        if let Err(reason) = result {
            return self.module_failed(module, reason);
        }
        self.modules[module].remaining = self.modules[module].remaining.saturating_sub(1);
        if self.modules[module].remaining == 0 {
            self.start_module(module)?;
        }
        Ok(())
    }
    pub(crate) fn module_failed(&mut self, module: usize, reason: JsValue) -> Result<(), JsError> {
        self.modules[module].status = Status::Errored;
        self.modules[module].error = Some(reason.clone());
        if let Some(promise) = self.modules[module].promise.clone() {
            self.reject_promise(promise, reason)?;
        }
        Ok(())
    }
    pub(crate) fn module_completed(&mut self, module: usize) -> Result<JsValue, JsError> {
        self.modules[module].status = Status::Evaluated;
        self.module_namespace(module)
    }
    fn start_module(&mut self, module: usize) -> Result<(), JsError> {
        if self.modules[module].status != Status::Evaluating {
            return Ok(());
        }
        let record = self.modules[module].clone();
        let promise = record
            .promise
            .ok_or_else(|| JsError::new(ErrorKind::Error, "module evaluation promise"))?;
        let mut frame = super::execution::Frame::new(
            record.code,
            record.environment,
            true,
            record.url.clone(),
            Span::default(),
        );
        frame.source = record.url;
        frame.module = Some(module);
        let id = self.allocate(
            ObjectKind::AsyncTask(Box::new(super::execution::AsyncData {
                frame: Some(frame),
                promise,
                finished: false,
                module: Some(module),
            })),
            None,
        )?;
        self.resume_async(id, None)
    }
    pub fn evaluate_module_in_task(&mut self, url: &str) -> Result<JsValue, JsError> {
        let id = self
            .module_urls
            .get(url)
            .copied()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "module not loaded"))?;
        self.link_module(id)?;
        self.evaluate_module(id, &mut Vec::new())
    }
    pub fn eval_module(&mut self, url: &str) -> Result<JsValue, JsError> {
        self.begin_task();
        self.evaluate_module_in_task(url)
    }
    pub(crate) fn dynamic_import(&mut self, specifier: JsValue) -> Result<JsValue, JsError> {
        let output = self.promise()?;
        let result = (|| {
            let specifier = self.to_string(specifier)?;
            let referrer = self.source_url.clone();
            let url = resolve_module_url(&referrer, &specifier)?;
            if self.module_urls.contains_key(&url) {
                self.evaluate_module_in_task(&url)
            } else if let Some(promise) = self.module_loads.get(&url) {
                Ok(promise.clone())
            } else {
                if self.module_loads.len() >= 128 {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "module load quota"));
                }
                let promise = self.with_host(|host, vm| host.import_module(vm, &url, &referrer))?;
                self.reserve_memory(url.len() + 64)?;
                self.module_loads.insert(url, promise.clone());
                Ok(promise)
            }
        })();
        match result {
            Ok(promise) => {
                self.resolve_promise(output.clone(), promise)?;
                Ok(output)
            }
            Err(error) if error.is_resource_limit() => Err(error),
            Err(error) => {
                let reason = if let Some(value) = error.thrown {
                    value
                } else {
                    self.error_value(error)?
                };
                self.reject_promise(output.clone(), reason)?;
                Ok(output)
            }
        }
    }
    pub(crate) fn import_meta(&mut self) -> Result<JsValue, JsError> {
        let module = self
            .current_module
            .ok_or_else(|| JsError::new(ErrorKind::SyntaxError, "import.meta outside a module"))?;
        if let Some(value) = &self.modules[module].meta {
            return Ok(value.clone());
        }
        let value = JsValue::Object(self.allocate(ObjectKind::Ordinary, None)?);
        self.set(value.clone(), "url", JsValue::string(&self.source_url))?;
        self.modules[module].meta = Some(value.clone());
        Ok(value)
    }
    pub(crate) fn module_roots(&self) -> Vec<JsValue> {
        self.modules
            .iter()
            .flat_map(|m| {
                m.namespace
                    .iter()
                    .chain(m.meta.iter())
                    .chain(
                        m.link_error
                            .iter()
                            .filter_map(|error| error.thrown.as_ref()),
                    )
                    .chain(m.promise.iter())
                    .chain(m.error.iter())
                    .cloned()
            })
            .collect()
    }
}
