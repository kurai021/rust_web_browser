//! Capped CORS module graph loading on the transport executor, never on the UI.
use crate::{
    http::{self, Request},
    policy::Csp,
    *,
};
use dom_bindings::js;
use std::collections::{HashSet, VecDeque};
#[derive(Debug, Clone)]
pub struct ModuleSource {
    pub url: String,
    pub base: String,
    pub source: String,
}
#[derive(Debug, Clone)]
pub struct ModuleGraph {
    pub entry: String,
    pub sources: Vec<ModuleSource>,
    pub aliases: Vec<(String, String)>,
}
pub async fn load_graph(
    client: &net::Client,
    entry: Url,
    document: &Url,
    policy: &Csp,
    nonce: Option<String>,
    inline: Option<String>,
) -> Result<ModuleGraph> {
    load_graph_cached(client, entry, document, policy, nonce, inline, &[]).await
}
pub async fn load_graph_cached(
    client: &net::Client,
    entry: Url,
    document: &Url,
    policy: &Csp,
    nonce: Option<String>,
    inline: Option<String>,
    known: &[String],
) -> Result<ModuleGraph> {
    let mut queue = VecDeque::from([entry.clone()]);
    let mut seen = known.iter().cloned().collect::<HashSet<_>>();
    let initial = seen.len();
    let mut fetched = HashSet::new();
    let mut graph = ModuleGraph {
        entry: entry.to_string(),
        sources: Vec::new(),
        aliases: Vec::new(),
    };
    let mut bytes = 0usize;
    let start = tokio::time::Instant::now();
    while let Some(url) = queue.pop_front() {
        if !seen.insert(url.to_string()) {
            continue;
        }
        if seen.len().saturating_sub(initial) > 64 {
            return Err(ApiError::new(
                "QuotaExceededError",
                "module graph request quota",
            ));
        }
        let remaining = std::time::Duration::from_secs(60).saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(ApiError::new("TimeoutError", "module graph deadline"));
        }
        let (key, base, source) = if url == entry && inline.is_some() {
            (
                url.to_string(),
                document.to_string(),
                inline.clone().unwrap_or_default(),
            )
        } else {
            let mut request = Request::get(url.clone());
            request.timeout_ms = remaining.as_millis().min(60_000) as u64;
            let response = http::fetch_resource(
                client,
                document,
                policy,
                request,
                "script-src",
                if url == entry { nonce.as_deref() } else { None },
            )
            .await?;
            if !(200..300).contains(&response.status) {
                return Err(ApiError::new("TypeError", "module HTTP status"));
            }
            let mime = response.headers.get("content-type").unwrap_or_default();
            let mime = mime
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !matches!(
                mime.as_str(),
                "text/javascript"
                    | "application/javascript"
                    | "text/ecmascript"
                    | "application/ecmascript"
                    | "application/x-javascript"
            ) {
                return Err(ApiError::new("TypeError", "module MIME rejected"));
            }
            let final_url = response.url.to_string();
            if url != response.url {
                graph.aliases.push((url.to_string(), final_url.clone()));
            }
            let source = String::from_utf8_lossy(&response.body)
                .trim_start_matches('\u{feff}')
                .to_owned();
            (final_url.clone(), final_url, source)
        };
        if !fetched.insert(key.clone()) {
            continue;
        }
        bytes = bytes.saturating_add(source.len());
        if bytes > MAX_BODY {
            return Err(ApiError::new(
                "QuotaExceededError",
                "module graph source bytes",
            ));
        }
        let program = js::parse_module(&source, js::ParseOpts::default())
            .map_err(|e| ApiError::new("SyntaxError", e.to_string()))?;
        for specifier in program.requests {
            let target = js::resolve_module_url(&base, &specifier)
                .map_err(|e| ApiError::new("TypeError", e.to_string()))?;
            let target = web_url(document, &target)?;
            queue.push_back(target);
        }
        graph.sources.push(ModuleSource {
            url: key,
            base,
            source,
        });
    }
    Ok(graph)
}
