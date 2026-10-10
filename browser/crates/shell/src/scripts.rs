//! Classic blocking/defer script loader using the existing parser and transport.
use crate::page::{load_page_from_document, Page};
use css::Environment;
use dom_bindings::{policy::Policy, DomRuntime};
use html::{Document, NodeId};
use js::{ErrorKind, JsError};
use net::{Client, Fetched};
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use url::Url;

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
    pub runtime: DomRuntime,
    pub page: Arc<Page>,
    pub errors: Vec<String>,
}
impl LoadedScripts {
    pub fn runtime_environment(&self) -> Environment {
        self.runtime.current_environment()
    }
    pub fn set_navigation(&mut self, url: Url) {
        if matches!(url.scheme(), "http" | "https") {
            self.runtime.request_navigation(url);
        }
    }
}
struct Loader<'a> {
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
        if document.get_attribute(node, "nomodule").is_some() { /* Level 1 supports classic scripts, not modules. */
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
                fetched = self.client.fetch_script(&url, MAX_SCRIPT_BYTES - self.bytes, &allow) => fetched,
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
            String::from_utf8_lossy(&fetched.bytes).into_owned()
        } else {
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
}

pub async fn load_scripts(
    client: &Client,
    fetched: Fetched,
    environment: Environment,
    active: Option<(Arc<AtomicU64>, u64)>,
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
    let mut runtime = DomRuntime::new(
        initial,
        fetched.url.clone(),
        Vec::new(),
        environment,
        policy.clone(),
    )?;
    runtime.vm.limits.navigation = active.clone();
    let mut loader = Loader {
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
                let is_deferred = document.get_attribute(node, "src").is_some()
                    && (document.get_attribute(node, "defer").is_some()
                        || document.get_attribute(node, "async").is_some());
                if is_deferred {
                    deferred.push(node);
                } else {
                    match loader.source(&document, node).await {
                        Ok(Some(source)) => {
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
        match loader.source(&runtime.document(), node).await {
            Ok(Some(source)) => {
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
    })
}
