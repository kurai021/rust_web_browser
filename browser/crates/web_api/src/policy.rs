//! Web requests reuse the DOM's single script/resource CSP implementation.
use crate::*;
#[derive(Debug, Clone, Default)]
pub struct Csp {
    pub inner: dom_bindings::policy::Policy,
}
impl Csp {
    pub fn parse(headers: &[String]) -> Self {
        Self {
            inner: dom_bindings::policy::Policy::from_sources(headers),
        }
    }
    pub fn eval_allowed(&self) -> bool {
        self.inner.eval_allowed()
    }
    pub fn inline_allowed(&self, source: &str, nonce: Option<&str>) -> bool {
        self.inner.inline_allowed(source, nonce)
    }
    pub fn allows(&self, kind: &str, document: &Url, resource: &Url, nonce: Option<&str>) -> bool {
        self.inner.resource_allowed(kind, document, resource, nonce)
    }
}
