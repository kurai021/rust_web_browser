//! HTTPS fetch client (plan/04 §4.3.2–§4.3.5).
//!
//! `reqwest` + `rustls` + `tokio` do the transport (HTTP/1.1 + HTTP/2,
//! system + bundled roots, gzip/br/zstd). This module adds the product
//! semantics: scheme gating, HSTS upgrade, https-first fallback,
//! redirect caps, body caps (zip-bomb guard), timeouts, progress and
//! typed errors. Dropping the fetch future cancels it (shell aborts the
//! task on stop/tab-close).

use std::sync::Mutex;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{CONTENT_TYPE, STRICT_TRANSPORT_SECURITY, USER_AGENT};
use url::Url;

use crate::error::map_reqwest;
use crate::{Error, HstsStore};

/// Default caps and timeouts (plan/04 §4.3.2).
pub const DEFAULT_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_MAX_REDIRECTS: usize = 10;

/// Fixed product User-Agent (plan/04 §4.3.2, §4.3.5): no extra fingerprint.
pub const USER_AGENT_VALUE: &str = concat!("browser/", env!("CARGO_PKG_VERSION"));

/// Tunables for [`Client`]. All fields have spec-backed defaults.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Max decoded body bytes kept; beyond that → [`Error::BodyTooLarge`].
    pub max_body_bytes: usize,
    /// TCP connect deadline.
    pub connect_timeout: Duration,
    /// Deadline for response headers (TTFB).
    pub first_byte_timeout: Duration,
    /// Deadline for the whole request.
    pub total_timeout: Duration,
    /// Max redirect hops followed.
    pub max_redirects: usize,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            first_byte_timeout: DEFAULT_FIRST_BYTE_TIMEOUT,
            total_timeout: DEFAULT_TOTAL_TIMEOUT,
            max_redirects: DEFAULT_MAX_REDIRECTS,
        }
    }
}

/// Download progress reports for the shell progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Decoded bytes received so far.
    pub downloaded: usize,
    /// `Content-Length` when the server sent one.
    pub total: Option<u64>,
}

/// A completed navigation fetch.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// Final URL after redirects (and any HSTS upgrade).
    pub url: Url,
    /// HTTP status code (2xx; anything else is [`Error::HttpStatus`]).
    pub status: u16,
    /// Response `Content-Type`, if present.
    pub content_type: Option<String>,
    /// Decoded body bytes (capped by [`FetchOptions::max_body_bytes`]).
    pub bytes: Vec<u8>,
}

/// HTTPS client with HSTS state.
pub struct Client {
    inner: reqwest::Client,
    hsts: Mutex<HstsStore>,
    options: FetchOptions,
}

impl Client {
    /// Build a client. Fails only when the TLS stack cannot initialize.
    pub fn new(options: FetchOptions) -> Result<Self, Error> {
        let inner = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(options.max_redirects))
            .connect_timeout(options.connect_timeout)
            .timeout(options.total_timeout)
            .cookie_store(true)
            // System + bundled (Mozilla) roots come from the
            // `rustls-tls-native-roots` + `rustls-tls-webpki-roots` features
            // (both default to enabled, plan/04 §4.3.3).
            .build()
            .map_err(|e| Error::Transport(format!("TLS init failed: {e}")))?;
        Ok(Self {
            inner,
            hsts: Mutex::new(HstsStore::new()),
            options,
        })
    }

    /// Access the HSTS store (persistence wiring lands in Phase 9).
    pub fn hsts(&self) -> std::sync::MutexGuard<'_, HstsStore> {
        self.hsts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// GET `url`, streaming the body with `on_progress` per chunk.
    ///
    /// Cancellation: drop the future (the shell aborts the task).
    /// The callback must be `Send`: it is held across `.await` points on a
    /// multi-thread runtime.
    pub async fn fetch(
        &self,
        url: &Url,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<Fetched, Error> {
        self.fetch_limited(url, self.options.max_body_bytes, on_progress)
            .await
    }

    /// A stricter resource-specific decoded-body limit. Shares the existing
    /// transport, cookies, HSTS, deadlines and streaming cancellation path.
    pub async fn fetch_limited(
        &self,
        url: &Url,
        max_bytes: usize,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<Fetched, Error> {
        let max_bytes = max_bytes.min(self.options.max_body_bytes);
        let url = self.effective_url(url)?;
        let request = self
            .inner
            .get(url.clone())
            .header(USER_AGENT, USER_AGENT_VALUE);
        let response = tokio::time::timeout(self.options.first_byte_timeout, request.send())
            .await
            .map_err(|_| Error::Timeout("time to first byte exceeded".to_owned()))?
            .map_err(|e| map_reqwest(&e))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::HttpStatus(status));
        }
        let final_url = response.url().clone();
        self.note_hsts(&final_url, &response);
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let total = response.content_length();

        let mut bytes = Vec::new();
        let mut downloaded = 0usize;
        let mut stream = response.bytes_stream();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| map_reqwest(&e))?;
            downloaded += chunk.len();
            if downloaded > max_bytes {
                return Err(Error::BodyTooLarge { cap: max_bytes });
            }
            bytes.extend_from_slice(&chunk);
            on_progress(Progress { downloaded, total });
        }
        Ok(Fetched {
            url: final_url,
            status,
            content_type,
            bytes,
        })
    }

    /// GET `url`, falling back to plain `http://` when the `https://`
    /// attempt fails at connect (never on TLS errors) — plan/04 §4.3.3.
    ///
    /// Set `allow_downgrade` only for bare-host input (`example.com`);
    /// HSTS entries always win over the fallback.
    pub async fn fetch_with_http_fallback(
        &self,
        url: &Url,
        allow_downgrade: bool,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<Fetched, Error> {
        match self.fetch(url, on_progress).await {
            Ok(fetched) => Ok(fetched),
            Err(Error::Connect(_) | Error::Dns(_) | Error::Timeout(_))
                if allow_downgrade && url.scheme() == "https" && !self.is_hsts_host(url) =>
            {
                let mut plain = url.clone();
                if plain.set_scheme("http").is_ok() {
                    return self.fetch(&plain, on_progress).await;
                }
                self.fetch(url, on_progress).await
            }
            Err(other) => Err(other),
        }
    }

    /// Apply scheme gating + HSTS upgrade. Pure and unit-tested.
    fn effective_url(&self, url: &Url) -> Result<Url, Error> {
        match url.scheme() {
            "https" => Ok(url.clone()),
            "http" => {
                if self.is_hsts_host(url) {
                    let mut upgraded = url.clone();
                    upgraded
                        .set_scheme("https")
                        .map_err(|_| Error::InvalidUrl(url.as_str().to_owned()))?;
                    Ok(upgraded)
                } else {
                    Ok(url.clone())
                }
            }
            other => Err(Error::UnsupportedScheme(other.to_owned())),
        }
    }

    fn is_hsts_host(&self, url: &Url) -> bool {
        url.host_str().is_some_and(|host| {
            self.hsts()
                .is_https_only(host, std::time::SystemTime::now())
        })
    }

    fn note_hsts(&self, final_url: &Url, response: &reqwest::Response) {
        if final_url.scheme() != "https" {
            return;
        }
        let (Some(host), Some(value)) = (
            final_url.host_str(),
            response
                .headers()
                .get(STRICT_TRANSPORT_SECURITY)
                .and_then(|v| v.to_str().ok()),
        ) else {
            return;
        };
        self.hsts()
            .note_header(host, value, std::time::SystemTime::now());
    }
}
