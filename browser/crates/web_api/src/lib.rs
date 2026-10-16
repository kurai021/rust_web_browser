//! Closed, origin-scoped web services. The shell owns I/O tasks and UI prompts.
pub mod browsing;
pub mod canvas;
pub mod headers;
pub mod http;
pub mod policy;
pub mod runtime;
pub mod services;
pub mod storage;
pub mod streams;
pub use runtime::{
    Command as WebCommand, Completion as WebCompletion, Config as RuntimeConfig, PageRuntime,
};
use serde::{Deserialize, Serialize};
use url::Url;
pub const MAX_BODY: usize = 8 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("{name}: {message}")]
pub struct ApiError {
    pub name: String,
    pub message: String,
}
impl ApiError {
    pub fn new(name: &str, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
        }
    }
}
pub type Result<T> = std::result::Result<T, ApiError>;
pub fn origin(url: &Url) -> Result<String> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ApiError::new(
            "SecurityError",
            "opaque or unsupported origin",
        ));
    }
    Ok(url.origin().ascii_serialization())
}
pub fn same_origin(a: &Url, b: &Url) -> bool {
    origin(a).is_ok() && a.origin() == b.origin()
}
pub fn secure_context(url: &Url) -> bool {
    url.scheme() == "https"
        || url.scheme() == "http"
            && (url.host_str() == Some("localhost")
                || match url.host() {
                    Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                    Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                    _ => false,
                })
}
pub fn web_url(base: &Url, value: &str) -> Result<Url> {
    let url = base
        .join(value)
        .map_err(|_| ApiError::new("TypeError", "invalid URL"))?;
    origin(&url)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError::new("TypeError", "URL credentials are forbidden"));
    }
    Ok(url)
}
