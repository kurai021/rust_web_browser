//! Single-hop HTTP operations. Web policy owns redirect/CORS decisions.
use crate::{Client, Error};
use futures::StreamExt;
use url::Url;
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub url: Url,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub top: Url,
    pub credentials: bool,
    pub max_bytes: usize,
    pub timeout: std::time::Duration,
}
#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub url: Url,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}
impl Client {
    pub async fn exchange(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        let deadline =
            tokio::time::Instant::now() + request.timeout.min(self.options.total_timeout);
        let cap = request.max_bytes.min(self.options.max_body_bytes);
        let (mut head, mut stream) = self.exchange_stream(request).await?;
        while let Some(chunk) = tokio::time::timeout_at(deadline, stream.next())
            .await
            .map_err(|_| Error::Timeout("body deadline".into()))?
        {
            let chunk = chunk?;
            if head.body.len().saturating_add(chunk.len()) > cap {
                return Err(Error::BodyTooLarge { cap });
            }
            head.body.extend_from_slice(&chunk);
        }
        Ok(head)
    }
    /// One-hop decoded stream, with capped chunks and the same TLS/cookie policy.
    /// The caller owns the lifetime/deadline and drops the stream to cancel I/O.
    pub async fn exchange_stream(
        &self,
        request: HttpRequest,
    ) -> Result<
        (
            HttpResponse,
            futures::stream::BoxStream<'static, Result<Vec<u8>, Error>>,
        ),
        Error,
    > {
        let url = self.effective_url(&request.url)?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| Error::Transport("invalid method".into()))?;
        let mut builder = self
            .inner
            .request(method, url.clone())
            .header(reqwest::header::USER_AGENT, crate::client::USER_AGENT_VALUE)
            .timeout(request.timeout.min(self.options.total_timeout));
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if request.body.is_empty() && matches!(request.method.as_str(), "POST" | "PUT") {
            builder = builder.header(reqwest::header::CONTENT_LENGTH, "0");
        }
        if request.credentials {
            let cookie = self.cookies.header(&url, &request.top, false);
            if !cookie.is_empty() {
                builder = builder.header(reqwest::header::COOKIE, cookie);
            }
        }
        let response = tokio::time::timeout(
            request.timeout.min(self.options.first_byte_timeout),
            builder.body(request.body).send(),
        )
        .await
        .map_err(|_| Error::Timeout("response headers deadline".into()))?
        .map_err(|e| crate::error::map_reqwest(&e))?;
        self.note_hsts(&url, &response);
        if request.credentials {
            for header in response.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Ok(v) = header.to_str() {
                    self.cookies.set(&url, &request.top, v, false);
                }
            }
        }
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().into(), v.into())))
            .collect();
        let cap = request.max_bytes.min(self.options.max_body_bytes);
        let stream = response
            .bytes_stream()
            .map(move |chunk| {
                let chunk = chunk.map_err(|e| crate::error::map_reqwest(&e))?;
                if chunk.len() > cap {
                    return Err(Error::BodyTooLarge { cap });
                }
                Ok(chunk.to_vec())
            })
            .boxed();
        Ok((
            HttpResponse {
                url,
                status: status.as_u16(),
                status_text: status.canonical_reason().unwrap_or("").into(),
                headers,
                body: Vec::new(),
            },
            stream,
        ))
    }
}
