//! Fetch/XHR share one CORS implementation and the existing capped transport.
use crate::{headers::Headers, policy::Csp, *};
use net::{Client, HttpRequest};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Credentials {
    Omit,
    #[default]
    SameOrigin,
    Include,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Cors,
    SameOrigin,
    NoCors,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub url: Url,
    pub method: String,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub credentials: Credentials,
    pub mode: Mode,
    pub timeout_ms: u64,
    pub redirect: String,
}
impl Request {
    pub fn get(url: Url) -> Self {
        Self {
            url,
            method: "GET".into(),
            headers: Headers::default(),
            body: Vec::new(),
            credentials: Credentials::SameOrigin,
            mode: Mode::Cors,
            timeout_ms: 30_000,
            redirect: "follow".into(),
        }
    }
    pub fn validate(&mut self) -> Result<()> {
        origin(&self.url)?;
        let method = Headers::validate_name(&self.method)?;
        if matches!(method.as_str(), "connect" | "trace" | "track") {
            return Err(ApiError::new("TypeError", "forbidden method"));
        }
        if matches!(
            method.as_str(),
            "delete" | "get" | "head" | "options" | "post" | "put"
        ) {
            self.method = method.to_ascii_uppercase();
        }
        if matches!(self.method.as_str(), "GET" | "HEAD") && !self.body.is_empty() {
            return Err(ApiError::new("TypeError", "GET/HEAD body"));
        }
        if self.body.len() > MAX_BODY {
            return Err(ApiError::new("QuotaExceededError", "request body"));
        }
        if !self.url.username().is_empty() || self.url.password().is_some() {
            return Err(ApiError::new("TypeError", "URL credentials"));
        }
        self.headers
            .0
            .retain(|(k, _)| !Headers::request_forbidden(k));
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub url: Url,
    pub status: u16,
    pub status_text: String,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub kind: String,
    pub redirected: bool,
}
pub(crate) fn check_allow(
    response: &net::HttpResponse,
    source: &str,
    credentials: bool,
) -> Result<()> {
    if response
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("access-control-allow-origin"))
        .count()
        != 1
    {
        return Err(ApiError::new(
            "TypeError",
            "CORS origin header must be singular",
        ));
    }
    let allow = response.header("access-control-allow-origin");
    if allow != Some(source) && !(allow == Some("*") && !credentials) {
        return Err(ApiError::new("TypeError", "CORS origin denied"));
    }
    if credentials && response.header("access-control-allow-credentials") != Some("true") {
        return Err(ApiError::new("TypeError", "CORS credentials denied"));
    }
    if credentials
        && response
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("access-control-allow-credentials"))
            .count()
            != 1
    {
        return Err(ApiError::new(
            "TypeError",
            "CORS credentials header must be singular",
        ));
    }
    Ok(())
}
fn tokens(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}
pub async fn fetch(
    client: &Client,
    document: &Url,
    policy: &Csp,
    mut request: Request,
) -> Result<Response> {
    request.validate()?;
    let source = origin(document)?;
    let started = tokio::time::Instant::now();
    let duration = std::time::Duration::from_millis(request.timeout_ms.clamp(1, 60_000));
    let mut redirected = false;
    for hop in 0..=10 {
        request.url = client
            .resource_url(&request.url)
            .map_err(|e| ApiError::new("TypeError", e.to_string()))?;
        if !policy.allows("connect-src", document, &request.url, None) {
            return Err(ApiError::new(
                "TypeError",
                "CSP or active mixed content blocked request",
            ));
        }
        let cross = !same_origin(document, &request.url);
        if cross && request.mode == Mode::SameOrigin {
            return Err(ApiError::new("TypeError", "same-origin mode"));
        }
        let credentials = request.credentials == Credentials::Include
            || request.credentials == Credentials::SameOrigin && !cross;
        let unsafe_headers = request
            .headers
            .entries()
            .into_iter()
            .filter(|(k, v)| !Headers::safelisted(k, v))
            .map(|(k, _)| k)
            .collect::<Vec<_>>();
        let unsafe_method = !matches!(request.method.as_str(), "GET" | "HEAD" | "POST");
        if request.mode == Mode::NoCors && (unsafe_method || !unsafe_headers.is_empty()) {
            return Err(ApiError::new(
                "TypeError",
                "no-cors request is not safelisted",
            ));
        }
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(ApiError::new("TimeoutError", "request deadline"));
        }
        if cross && request.mode == Mode::Cors && (unsafe_method || !unsafe_headers.is_empty()) {
            let mut headers = vec![
                ("origin".into(), source.clone()),
                (
                    "access-control-request-method".into(),
                    request.method.clone(),
                ),
            ];
            if !unsafe_headers.is_empty() {
                headers.push((
                    "access-control-request-headers".into(),
                    unsafe_headers.join(", "),
                ));
            }
            let response = client
                .exchange(HttpRequest {
                    url: request.url.clone(),
                    method: "OPTIONS".into(),
                    headers,
                    body: Vec::new(),
                    top: document.clone(),
                    credentials: false,
                    max_bytes: 64 * 1024,
                    timeout: remaining,
                })
                .await
                .map_err(transport_error)?;
            if !(200..300).contains(&response.status) {
                return Err(ApiError::new("TypeError", "CORS preflight status"));
            }
            check_allow(&response, &source, credentials)?;
            let methods = response
                .header("access-control-allow-methods")
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            for method in &methods {
                Headers::validate_name(method)?;
            }
            let allowed = tokens(response.header("access-control-allow-headers"));
            if unsafe_method
                && !methods.contains(&request.method)
                && !(!credentials && methods.iter().any(|m| m == "*"))
            {
                return Err(ApiError::new("TypeError", "CORS method denied"));
            }
            if unsafe_headers.iter().any(|h| {
                !allowed.contains(h)
                    && !(!credentials && h != "authorization" && allowed.iter().any(|v| v == "*"))
            }) {
                return Err(ApiError::new("TypeError", "CORS header denied"));
            }
        }
        let mut headers = request.headers.0.clone();
        if cross || !matches!(request.method.as_str(), "GET" | "HEAD") {
            headers.push(("origin".into(), source.clone()));
        }
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(ApiError::new("TimeoutError", "request deadline"));
        }
        let response = client
            .exchange(HttpRequest {
                url: request.url.clone(),
                method: request.method.clone(),
                headers,
                body: request.body.clone(),
                top: document.clone(),
                credentials,
                max_bytes: MAX_BODY,
                timeout: remaining,
            })
            .await
            .map_err(transport_error)?;
        if cross && request.mode == Mode::Cors {
            check_allow(&response, &source, credentials)?;
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            if request.redirect == "error" {
                return Err(ApiError::new("TypeError", "redirect forbidden"));
            }
            if request.redirect == "manual" {
                return Ok(Response {
                    url: request.url,
                    status: 0,
                    status_text: String::new(),
                    headers: Headers::default(),
                    body: Vec::new(),
                    kind: "opaqueredirect".into(),
                    redirected: false,
                });
            }
            if hop == 10 {
                return Err(ApiError::new("TypeError", "redirect limit"));
            }
            let target = web_url(
                &request.url,
                response
                    .header("location")
                    .ok_or_else(|| ApiError::new("TypeError", "redirect Location"))?,
            )?;
            if !same_origin(&request.url, &target) {
                request.headers.delete("authorization");
            }
            if response.status == 303 && request.method != "HEAD"
                || matches!(response.status, 301 | 302) && request.method == "POST"
            {
                request.method = "GET".into();
                request.body.clear();
                request.headers.delete("content-type");
            }
            request.url = target;
            redirected = true;
            continue;
        }
        let opaque = cross && request.mode == Mode::NoCors;
        let exposed = tokens(response.header("access-control-expose-headers"));
        let mut headers = Headers::default();
        if !opaque {
            for (k, v) in response.headers {
                if matches!(k.as_str(), "set-cookie" | "set-cookie2") {
                    continue;
                }
                if !cross
                    || matches!(
                        k.as_str(),
                        "cache-control"
                            | "content-length"
                            | "content-language"
                            | "content-type"
                            | "expires"
                            | "last-modified"
                            | "pragma"
                    )
                    || exposed.contains(&k)
                    || exposed.iter().any(|s| s == "*") && !credentials
                {
                    headers.append(&k, &v)?;
                }
            }
        }
        return Ok(Response {
            url: response.url,
            status: if opaque { 0 } else { response.status },
            status_text: if opaque {
                String::new()
            } else {
                response.status_text
            },
            headers,
            body: if opaque { Vec::new() } else { response.body },
            kind: if opaque {
                "opaque"
            } else if cross {
                "cors"
            } else {
                "basic"
            }
            .into(),
            redirected: redirected && !opaque,
        });
    }
    Err(ApiError::new("TypeError", "redirect limit"))
}
fn transport_error(error: net::Error) -> ApiError {
    ApiError::new(
        if matches!(error, net::Error::Timeout(_)) {
            "TimeoutError"
        } else {
            "TypeError"
        },
        error.to_string(),
    )
}
