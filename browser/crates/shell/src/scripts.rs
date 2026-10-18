//! Classic blocking/defer script loader using the existing parser and transport.
use crate::page::{load_page_from_document, Page};
use css::Environment;
use dom_bindings::policy::Policy;
use html::{Document, NodeId};
use js::{ErrorKind, JsError};
use net::{Client, Fetched};
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use url::Url;
use web_api::{PageRuntime, RuntimeConfig};

const MAX_SCRIPT_REQUESTS: usize = 64;
const MAX_SCRIPT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) async fn wait_cancelled(active: &Option<(Arc<AtomicU64>, u64)>) {
    let Some((token, id)) = active else {
        return std::future::pending().await;
    };
    while token.load(Ordering::Relaxed) == *id {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
pub struct LoadedScripts {
    pub runtime: PageRuntime,
    pub page: Arc<Page>,
    pub errors: Vec<String>,
    headless_services: Option<web_api::services::Services>,
}
impl LoadedScripts {
    pub fn register_images(&mut self) -> Result<(), JsError> {
        let document = self.runtime.document();
        for node in document
            .get_elements_by_tag_name("img")
            .into_iter()
            .take(crate::images::MAX_IMAGE_REQUESTS)
        {
            let Some(url) = document
                .get_attribute(node, "src")
                .and_then(|src| self.runtime.url().join(src).ok())
            else {
                continue;
            };
            let Some(image) = self.page.image_cache.get(&url) else {
                continue;
            };
            let Some(image) = image else {
                self.runtime
                    .image_error(node, &url, document.image_epoch(node))?;
                continue;
            };
            let tainted = self
                .page
                .image_taint
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&url)
                .copied()
                .unwrap_or_else(|| {
                    url.scheme() != "data" && !web_api::same_origin(&self.runtime.url(), &url)
                });
            self.runtime.image_at(
                node,
                &url,
                document.image_epoch(node),
                web_api::runtime::CanvasImage {
                    width: image.width,
                    height: image.height,
                    pixels: image.rgba.clone(),
                    tainted,
                },
            )?;
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Arc<Page> {
        let mut page = (*self.page).clone();
        page.document = Arc::new(self.runtime.document());
        page.url = self.runtime.url();
        Arc::new(page)
    }
    pub fn runtime_environment(&self) -> Environment {
        self.runtime.current_environment()
    }
    pub fn set_navigation(&mut self, url: Url) {
        if matches!(url.scheme(), "http" | "https") {
            if let Err(error) = self.runtime.navigate(url, false) {
                self.errors.push(error.to_string());
            }
        }
    }
}
pub fn headless_config(
    client: &Client,
    directory: Option<std::path::PathBuf>,
) -> web_api::Result<RuntimeConfig> {
    let profile = Arc::new(web_api::storage::Profile::open(directory.clone())?);
    if let Some(dir) = directory {
        client
            .cookies
            .load(&dir.join("cookies.json"))
            .map_err(|e| web_api::ApiError::new("DataError", e.to_string()))?;
        *client.hsts() = net::HstsStore::load_from(&dir.join("hsts.json"));
    }
    Ok(RuntimeConfig {
        profile,
        cookies: client.cookies.clone(),
        session: format!("headless-{}", std::process::id()),
        ..Default::default()
    })
}
/// Bounded headless service checkpoint. Native permissions are denied explicitly.
pub async fn settle(
    client: Arc<Client>,
    loaded: &mut LoadedScripts,
    budget: std::time::Duration,
) -> Result<(), JsError> {
    let mut services = loaded.headless_services.take().unwrap_or_else(|| {
        web_api::services::Services::new(client, tokio::runtime::Handle::current())
    });
    let start = std::time::Instant::now();
    loop {
        for command in loaded.runtime.commands() {
            match services
                .dispatch(command)
                .map_err(|e| JsError::new(ErrorKind::Error, e.to_string()))?
            {
                Some(web_api::WebCommand::Permission { id, name, .. }) => {
                    loaded
                        .runtime
                        .complete(web_api::WebCompletion::Permission {
                            id,
                            name,
                            decision: "denied".into(),
                        })?
                }
                Some(web_api::WebCommand::Clipboard { id, .. }) => {
                    loaded.runtime.complete(web_api::WebCompletion::Clipboard {
                        id,
                        result: Err(web_api::ApiError::new(
                            "NotAllowedError",
                            "headless clipboard disabled",
                        )),
                    })?
                }
                Some(web_api::WebCommand::Notice(message)) => {
                    loaded.runtime.handle().console(message)
                }
                Some(web_api::WebCommand::Popup { .. }) => loaded
                    .runtime
                    .handle()
                    .console("Headless popup blocked".into()),
                _ => {}
            }
        }
        for _ in 0..128 {
            let Some(completion) = services.poll() else {
                break;
            };
            loaded.runtime.complete(completion)?;
        }
        loaded.runtime.poll_timers()?;
        if start.elapsed() >= budget
            || services.is_idle() && !loaded.runtime.has_pending() && !loaded.runtime.has_timers()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    loaded.headless_services = Some(services);
    loaded.page = loaded.snapshot();
    Ok(())
}
struct Loader<'a> {
    source_url: Url,
    client: &'a Client,
    document: &'a Url,
    policy: Policy,
    requests: usize,
    bytes: usize,
    active: Option<(Arc<AtomicU64>, u64)>,
}
impl Loader<'_> {
    async fn source(
        &mut self,
        document: &Document,
        node: NodeId,
    ) -> Result<Option<String>, JsError> {
        let script_type = document
            .get_attribute(node, "type")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            script_type.as_str(),
            "" | "text/javascript"
                | "application/javascript"
                | "text/ecmascript"
                | "application/ecmascript"
        ) {
            return Ok(None);
        }
        if document.get_attribute(node, "nomodule").is_some() {
            return Ok(None);
        }
        let nonce = document.get_attribute(node, "nonce");
        let source = if let Some(src) = document.get_attribute(node, "src") {
            let base = document
                .get_elements_by_tag_name("base")
                .into_iter()
                .find_map(|id| {
                    document
                        .get_attribute(id, "href")
                        .and_then(|s| self.document.join(s).ok())
                })
                .unwrap_or_else(|| self.document.clone());
            let url = base
                .join(src)
                .map_err(|_| JsError::new(ErrorKind::Error, "invalid script URL"))?;
            if self.requests >= MAX_SCRIPT_REQUESTS || self.bytes >= MAX_SCRIPT_BYTES {
                return Err(JsError::new(ErrorKind::MemoryLimit, "page script budget"));
            }
            self.requests += 1;
            let allow =
                |resource: &Url| self.policy.external_allowed(self.document, resource, nonce);
            let fetched = tokio::select! {
                fetched = self.client.fetch_script_for(self.document, &url, MAX_SCRIPT_BYTES - self.bytes, &allow) => fetched,
                _ = wait_cancelled(&self.active) => return Err(JsError::new(ErrorKind::Cancelled, "navigation cancelled")),
            };
            let fetched = fetched.map_err(|e| JsError::new(ErrorKind::Error, e.to_string()))?;
            if fetched
                .content_type
                .as_deref()
                .is_some_and(|t| t.starts_with("image/") || t.starts_with("text/html"))
            {
                return Err(JsError::new(ErrorKind::Error, "script MIME rejected"));
            }
            self.bytes += fetched.bytes.len();
            self.source_url = fetched.url.clone();
            String::from_utf8_lossy(&fetched.bytes).into_owned()
        } else {
            self.source_url = self.document.clone();
            let source = document.text_content(node);
            if !self.policy.inline_allowed(&source, nonce) {
                return Err(JsError::new(ErrorKind::Error, "CSP blocks inline script"));
            }
            self.bytes = self.bytes.saturating_add(source.len());
            if self.bytes > MAX_SCRIPT_BYTES {
                return Err(JsError::new(ErrorKind::MemoryLimit, "page script budget"));
            }
            source
        };
        Ok(Some(source))
    }
    async fn module(
        &mut self,
        runtime: &mut PageRuntime,
        document: &Document,
        node: NodeId,
    ) -> Result<(), JsError> {
        let nonce = document.get_attribute(node, "nonce").map(str::to_owned);
        let (source, url) = if let Some(src) = document.get_attribute(node, "src") {
            (
                None,
                self.document
                    .join(src)
                    .map_err(|e| JsError::new(ErrorKind::TypeError, e.to_string()))?,
            )
        } else {
            let source = document.text_content(node);
            if !self.policy.inline_allowed(&source, nonce.as_deref()) {
                return Err(JsError::new(ErrorKind::Error, "CSP blocks inline module"));
            }
            let mut key = self.document.clone();
            key.set_fragment(Some(&format!("~inline-module-{node}")));
            (Some(source), key)
        };
        if runtime.vm.has_module(url.as_str()) {
            runtime.vm.begin_task();
            runtime.vm.evaluate_module_in_task(url.as_str())?;
            return Ok(());
        }
        if self.requests >= MAX_SCRIPT_REQUESTS {
            return Err(JsError::new(ErrorKind::MemoryLimit, "page module requests"));
        }
        let policy = web_api::policy::Csp {
            inner: self.policy.clone(),
        };
        let known = runtime.vm.loaded_module_urls();
        let graph = tokio::select! {graph=web_api::modules::load_graph_cached(self.client,url,self.document,&policy,nonce,source,&known)=>graph.map_err(|e|JsError::new(ErrorKind::Error,e.to_string()))?,_=wait_cancelled(&self.active)=>return Err(JsError::new(ErrorKind::Cancelled,"module navigation cancelled"))};
        self.requests += graph.sources.len();
        self.bytes = self
            .bytes
            .saturating_add(graph.sources.iter().map(|s| s.source.len()).sum::<usize>());
        if self.bytes > MAX_SCRIPT_BYTES || self.requests > MAX_SCRIPT_REQUESTS {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "page module source budget",
            ));
        }
        runtime.eval_module_graph(&graph)?;
        Ok(())
    }
}

pub async fn load_scripts(
    client: &Client,
    fetched: Fetched,
    environment: Environment,
    active: Option<(Arc<AtomicU64>, u64)>,
) -> Result<LoadedScripts, JsError> {
    let config = RuntimeConfig {
        cookies: client.cookies.clone(),
        ..Default::default()
    };
    load_scripts_configured(client, fetched, environment, active, config).await
}
pub async fn load_scripts_configured(
    client: &Client,
    fetched: Fetched,
    environment: Environment,
    active: Option<(Arc<AtomicU64>, u64)>,
    config: RuntimeConfig,
) -> Result<LoadedScripts, JsError> {
    let mut parser = html::Parser::new(
        fetched.url.clone(),
        Box::new(html::NullSink),
        html::ParseOpts {
            scripting_enabled: true,
            ..Default::default()
        },
    );
    parser.push(&fetched.bytes);
    let initial = parser.snapshot().unwrap_or_default();
    let policy = Policy::parse(&fetched.script_policies, &initial);
    let mut runtime = PageRuntime::new(
        initial,
        fetched.url.clone(),
        Vec::new(),
        environment,
        policy.clone(),
        config,
    )?;
    runtime.vm.limits.navigation = active.clone();
    // The shell's JS worker and explicit headless runner use 8 MiB stacks.
    runtime.vm.limits.max_native_stack_bytes = 4 * 1024 * 1024;
    let mut loader = Loader {
        source_url: fetched.url.clone(),
        client,
        document: &fetched.url,
        policy,
        requests: 0,
        bytes: 0,
        active: active.clone(),
    };
    let mut seen = HashSet::new();
    let mut deferred = Vec::new();
    let mut errors = Vec::new();
    let mut ended = false;
    loop {
        if parser.is_suspended() {
            let document = parser.snapshot().unwrap_or_else(|| runtime.document());
            runtime.replace_document(document.clone());
            runtime.set_policy(Policy::parse(&fetched.script_policies, &document));
            loader.policy = Policy::parse(&fetched.script_policies, &document);
            if let Some(node) = document
                .get_elements_by_tag_name("script")
                .into_iter()
                .find(|node| !seen.contains(node))
            {
                seen.insert(node);
                let is_module = document
                    .get_attribute(node, "type")
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("module"));
                let is_deferred = is_module
                    || document.get_attribute(node, "src").is_some()
                        && (document.get_attribute(node, "defer").is_some()
                            || document.get_attribute(node, "async").is_some());
                if is_deferred {
                    deferred.push(node);
                } else {
                    match loader.source(&document, node).await {
                        Ok(Some(source)) => {
                            runtime.vm.set_source_url(loader.source_url.as_str());
                            if let Err(error) = runtime.eval(&source) {
                                if error.is_resource_limit() {
                                    return Err(error);
                                }
                                errors.push(error.to_string());
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            if error.is_resource_limit() {
                                return Err(error);
                            }
                            errors.push(error.to_string());
                        }
                    }
                }
            }
            parser.replace_document(runtime.document());
            parser.resume();
        } else if !ended {
            parser.end_input();
            ended = true;
        } else {
            break;
        }
    }
    let document = parser.finish();
    runtime.replace_document(document.clone());
    let mut page = tokio::select! {
        page = load_page_from_document(client, fetched.clone(), document) => page,
        _ = wait_cancelled(&active) => return Err(JsError::new(ErrorKind::Cancelled, "navigation cancelled")),
    };
    runtime.stylesheets(page.stylesheets.clone());
    runtime.set_ready("interactive");
    for node in deferred {
        let document = runtime.document();
        if document
            .get_attribute(node, "type")
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("module"))
        {
            if let Err(error) = loader.module(&mut runtime, &document, node).await {
                if error.is_resource_limit() {
                    return Err(error);
                }
                errors.push(error.to_string());
            }
            continue;
        }
        match loader.source(&runtime.document(), node).await {
            Ok(Some(source)) => {
                runtime.vm.set_source_url(loader.source_url.as_str());
                if let Err(error) = runtime.eval(&source) {
                    if error.is_resource_limit() {
                        return Err(error);
                    }
                    errors.push(error.to_string());
                }
            }
            Ok(None) => {}
            Err(error) => {
                if error.is_resource_limit() {
                    return Err(error);
                }
                errors.push(error.to_string());
            }
        }
    }
    runtime.dispatch(0, "DOMContentLoaded", "")?;
    runtime.set_ready("complete");
    runtime.dispatch(0, "load", "")?;
    page.document = Arc::new(runtime.document());
    page.warnings.extend(errors.clone());
    runtime.gc_collect();
    Ok(LoadedScripts {
        runtime,
        page: Arc::new(page),
        errors,
        headless_services: None,
    })
}
