//! `net` — Phase 1 networking: WHATWG URLs, HTTPS fetch, HSTS.
//!
//! Spec: `plan/04-network-https.md`.
//! TCP/TLS/DNS are reused (`tokio`, `rustls`, system resolver); URL,
//! HTTP semantics, cache policy, cookies and HSTS are built here.

pub mod client;
pub mod error;
pub mod hsts;
pub mod url;

pub use client::{Client, FetchOptions, Fetched, Progress};
pub use error::{Error, TlsError};
pub use hsts::HstsStore;
pub use url::{classify_user_input, parse_url, UserInput};
