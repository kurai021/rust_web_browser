//! Page-owned HTML/CSS loading (plan/06, Phase 3). All fetch and parsing
//! runs on the existing network worker; the CSS crate never performs I/O.

use css::{Cascade, ComputedStyles, Environment, MediaQueryList, Rule, Stylesheet};
use html::{Document, NodeData};
use net::{Client, Fetched};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

const MAX_CSS_REQUESTS: usize = 64;
const MAX_CSS_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMPORT_DEPTH: usize = 5;

#[derive(Debug, Clone)]
pub struct Page {
    pub url: Url,
    pub status: u16,
    pub document: Arc<Document>,
    pub stylesheets: Vec<Stylesheet>,
    pub warnings: Vec<String>,
    pub(crate) font_cache: Arc<crate::fonts::FontCache>,
}
impl Page {
    pub fn computed_styles(&self, env: Environment) -> ComputedStyles {
        Cascade {
            environment: env,
            ..Cascade::default()
        }
        .compute(&self.document, &self.stylesheets)
    }
}

/// Load the document's CSS in document order, not request completion order.
/// Invalid/failed sheets are isolated; the remaining page stays navigable.
pub async fn load_page(client: &Client, fetched: Fetched) -> Page {
    let document = Arc::new(html::parse_full(
        &fetched.bytes,
        &fetched.url,
        html::ParseOpts::default(),
    ));
    let base = document
        .get_elements_by_tag_name("base")
        .into_iter()
        .find_map(|id| {
            document
                .get_attribute(id, "href")
                .and_then(|s| fetched.url.join(s).ok())
        })
        .unwrap_or_else(|| fetched.url.clone());
    let mut loader = Loader {
        client,
        document_url: &fetched.url,
        cache: BTreeMap::new(),
        requests: 0,
        total_bytes: 0,
        warnings: Vec::new(),
    };
    let mut sheets = Vec::new();
    let mut stack = vec![document.root()];
    while let Some(id) = stack.pop() {
        let NodeData::Element(el) = &document.get(id).data else {
            for &child in document.get(id).children.iter().rev() {
                stack.push(child);
            }
            continue;
        };
        let css_type = document
            .get_attribute(id, "type")
            .is_none_or(|s| s.is_empty() || s.eq_ignore_ascii_case("text/css"));
        let media = document.get_attribute(id, "media").unwrap_or("");
        let mut sheet = if el.tag_name == "style" && css_type {
            let raw = document.text_content(id);
            if loader.total_bytes.saturating_add(raw.len()) > MAX_CSS_TOTAL_BYTES {
                loader.warnings.push("embedded CSS budget exceeded".into());
                None
            } else {
                loader.total_bytes += raw.len();
                Some(css::parse_stylesheet(&raw, Some(&base)))
            }
        } else if el.tag_name == "link"
            && css_type
            && document.get_attribute(id, "disabled").is_none()
        {
            let rel = document.get_attribute(id, "rel").unwrap_or("");
            let stylesheet = rel
                .split_ascii_whitespace()
                .any(|s| s.eq_ignore_ascii_case("stylesheet"));
            let alternate = rel
                .split_ascii_whitespace()
                .any(|s| s.eq_ignore_ascii_case("alternate"));
            if stylesheet && !alternate {
                if let Some(url) = document
                    .get_attribute(id, "href")
                    .and_then(|s| css::stylesheet::resource_url(Some(&base), s))
                {
                    loader.fetch_sheet(&url).await
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(ref mut sheet) = sheet {
            let chain = sheet.base_url.iter().cloned().collect::<Vec<_>>();
            loader.fill_imports(sheet, 0, chain).await;
            if !media.trim().is_empty() {
                sheet.rules = vec![Rule::Media(
                    MediaQueryList::parse(media),
                    std::mem::take(&mut sheet.rules),
                )];
            }
        }
        if let Some(sheet) = sheet {
            sheets.push(sheet);
        }
        // Template contents are detached/inert; do not fetch their URLs.
        if el.tag_name != "template" {
            for &child in document.get(id).children.iter().rev() {
                stack.push(child);
            }
        }
    }
    Page {
        url: fetched.url.clone(),
        status: fetched.status,
        document,
        stylesheets: sheets,
        warnings: loader.warnings,
        font_cache: Arc::default(),
    }
}

struct Loader<'a> {
    client: &'a Client,
    document_url: &'a Url,
    cache: BTreeMap<Url, Stylesheet>,
    requests: usize,
    total_bytes: usize,
    warnings: Vec<String>,
}
impl Loader<'_> {
    async fn fetch_sheet(&mut self, url: &Url) -> Option<Stylesheet> {
        if let Some(sheet) = self.cache.get(url) {
            return Some(sheet.clone());
        }
        if self.requests >= MAX_CSS_REQUESTS
            || self.total_bytes >= MAX_CSS_TOTAL_BYTES
            || !allowed_subresource(self.document_url, url)
        {
            self.warnings.push(format!("CSS request rejected: {url}"));
            return None;
        }
        self.requests += 1;
        let cap = css::syntax::MAX_STYLESHEET_BYTES.min(MAX_CSS_TOTAL_BYTES - self.total_bytes);
        let fetched = match self.client.fetch_limited(url, cap, &mut |_| {}).await {
            Ok(f) => f,
            Err(err) => {
                self.warnings.push(format!("CSS load failed: {url}: {err}"));
                return None;
            }
        };
        if !allowed_subresource(self.document_url, &fetched.url) {
            self.warnings
                .push(format!("CSS redirect rejected: {}", fetched.url));
            return None;
        }
        if !fetched.content_type.as_deref().is_some_and(|t| {
            t.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("text/css")
        }) {
            self.warnings.push(format!("CSS MIME rejected: {url}"));
            return None;
        }
        self.total_bytes += fetched.bytes.len();
        let text = decode_css(&fetched.bytes, fetched.content_type.as_deref());
        let sheet = css::parse_stylesheet(&text, Some(&fetched.url));
        self.cache.insert(url.clone(), sheet.clone());
        Some(sheet)
    }

    fn fill_imports<'a>(
        &'a mut self,
        sheet: &'a mut Stylesheet,
        depth: usize,
        chain: Vec<Url>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            for rule in &mut sheet.rules {
                if let Rule::Import(import) = rule {
                    if depth >= MAX_IMPORT_DEPTH || chain.contains(&import.url) {
                        self.warnings
                            .push(format!("CSS import cycle/depth limit: {}", import.url));
                        continue;
                    }
                    if let Some(mut child) = self.fetch_sheet(&import.url).await {
                        if child
                            .base_url
                            .as_ref()
                            .is_some_and(|url| chain.contains(url))
                        {
                            self.warnings
                                .push(format!("CSS redirect import cycle: {}", import.url));
                            continue;
                        }
                        let mut next = chain.clone();
                        next.push(import.url.clone());
                        if let Some(base) = &child.base_url {
                            if !next.contains(base) {
                                next.push(base.clone());
                            }
                        }
                        self.fill_imports(&mut child, depth + 1, next).await;
                        import.sheet = Some(Box::new(child));
                    }
                }
            }
        })
    }
}

pub(crate) fn allowed_subresource(document: &Url, resource: &Url) -> bool {
    matches!(resource.scheme(), "http" | "https")
        && !(document.scheme() == "https" && resource.scheme() == "http")
}

fn decode_css(bytes: &[u8], content_type: Option<&str>) -> String {
    let transport = content_type.and_then(|s| {
        s.split(';').skip(1).find_map(|p| {
            p.trim()
                .split_once('=')
                .filter(|(name, _)| name.eq_ignore_ascii_case("charset"))
                .and_then(|(_, value)| {
                    encoding_rs::Encoding::for_label(value.trim_matches(['\'', '"']).as_bytes())
                })
        })
    });
    let charset = bytes.strip_prefix(b"@charset \"").and_then(|s| {
        s.iter()
            .position(|&b| b == b'"')
            .and_then(|i| encoding_rs::Encoding::for_label(&s[..i]))
    });
    let encoding = transport.or(charset).unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}
