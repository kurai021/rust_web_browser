//! Streaming top-level GETs and download redirects. No DOM or filesystem access.
use crate::{Client, Error, HttpRequest, HttpResponse};
use futures::stream::BoxStream;
use std::time::Duration;
use url::Url;
pub type BodyStream = BoxStream<'static, Result<Vec<u8>, Error>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheMode {
    #[default]
    Normal,
    Revalidate,
    Bypass,
}
impl Client {
    /// Follow a bounded GET redirect chain without buffering the response body.
    /// Headers are supplied by the trusted shell (Range/If-Range/cache controls).
    /// Cancelling or dropping the returned stream releases the transport permit.
    pub async fn open_get(
        &self,
        url: &Url,
        top: &Url,
        headers: Vec<(String, String)>,
        timeout: Duration,
    ) -> Result<(HttpResponse, BodyStream), Error> {
        self.open_get_kind(url, top, headers, timeout, true).await
    }
    pub async fn open_navigation(
        &self,
        url: &Url,
        headers: Vec<(String, String)>,
        timeout: Duration,
    ) -> Result<(HttpResponse, BodyStream), Error> {
        self.open_get_kind(url, url, headers, timeout, false).await
    }
    async fn open_get_kind(
        &self,
        url: &Url,
        top: &Url,
        headers: Vec<(String, String)>,
        timeout: Duration,
        download: bool,
    ) -> Result<(HttpResponse, BodyStream), Error> {
        let mut current = self.effective_url(url)?;
        let deadline = tokio::time::Instant::now() + timeout;
        for hop in 0..=self.options.max_redirects {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("GET redirect deadline".into()));
            }
            let (head, stream) = self
                .exchange_stream_kind(
                    HttpRequest {
                        url: current.clone(),
                        method: "GET".into(),
                        headers: headers.clone(),
                        body: Vec::new(),
                        top: top.clone(),
                        credentials: true,
                        max_bytes: 1024 * 1024,
                        timeout: remaining,
                    },
                    download,
                )
                .await?;
            if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
                if hop == self.options.max_redirects {
                    return Err(Error::TooManyRedirects);
                }
                let target = head
                    .header("location")
                    .ok_or_else(|| Error::Transport("Redirect has no Location".into()))?;
                current = self.effective_url(
                    &current
                        .join(target)
                        .map_err(|_| Error::InvalidUrl(target.into()))?,
                )?;
                drop(stream);
            } else {
                return Ok((head, stream));
            }
        }
        Err(Error::TooManyRedirects)
    }
    pub fn network_attempts(&self) -> u64 {
        self.attempts.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn document_byte_limit(&self) -> usize {
        self.options.max_body_bytes
    }
}
