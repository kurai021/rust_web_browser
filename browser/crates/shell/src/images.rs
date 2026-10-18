//! Deferred image loading. Network and bounded decoding stay off the UI.

use crate::page::{allowed_subresource, Page};
use base64::Engine;
use css::ComputedStyles;
use layout::{RenderKind, Replaced};
use net::Client;
use paint::images::MAX_IMAGE_BYTES;
use std::collections::HashSet;
use url::Url;

pub const MAX_IMAGE_REQUESTS: usize = 128;
const MAX_PAGE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
#[derive(Debug, Default)]
pub(crate) struct ImageRequests {
    attempted: HashSet<Url>,
    bytes: usize,
}
#[derive(Debug, Default)]
pub(crate) struct Icons {
    pub attempts: usize,
    pub cache: std::collections::HashMap<Url, Option<std::sync::Arc<layout::RasterImage>>>,
}

pub async fn load_images(
    client: &Client,
    page: &Page,
    styles: &ComputedStyles,
    mut on_ready: impl FnMut(Url, bool) + Send,
) {
    let tree = layout::build_render_tree(&page.document, styles);
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for node in &tree.nodes {
        if let RenderKind::Replaced(Replaced::Image { url: Some(url), .. }) = &node.kind {
            if seen.insert(url.clone()) {
                urls.push(url.clone());
            }
        }
        if let Some(css::style::BackgroundImage::Url(url)) = &node.style.background_image {
            if seen.insert(url.clone()) {
                urls.push(url.clone());
            }
        }
    }
    for url in urls.into_iter().take(MAX_IMAGE_REQUESTS) {
        // This serializes overlapping navigation/resize misses; it never runs
        // on the UI thread. Negative entries also avoid repeated bad decodes.
        let mut requests = page.image_requests.lock().await;
        if page.image_cache.get(&url).is_some() || requests.attempted.contains(&url) {
            continue;
        }
        if requests.attempted.len() >= MAX_IMAGE_REQUESTS || requests.bytes >= MAX_PAGE_IMAGE_BYTES
        {
            break;
        }
        let cap = MAX_IMAGE_BYTES.min(MAX_PAGE_IMAGE_BYTES - requests.bytes);
        let mut tainted = false;
        let bytes = if url.scheme() == "data" {
            if page
                .policy
                .resource_allowed("img-src", &page.url, &url, None)
            {
                data_image(&url).filter(|b| b.len() <= cap)
            } else {
                None
            }
        } else if allowed_subresource(&page.url, &url) {
            let allow = |target: &Url| {
                allowed_subresource(&page.url, target)
                    && page
                        .policy
                        .resource_allowed("img-src", &page.url, target, None)
            };
            match client.fetch_script_for(&page.url, &url, cap, &allow).await {
                Ok(fetched) if allowed_subresource(&page.url, &fetched.url) => {
                    tainted = !web_api::same_origin(&page.url, &url)
                        || !web_api::same_origin(&page.url, &fetched.url);
                    Some(fetched.bytes)
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(bytes) = &bytes {
            requests.bytes += bytes.len();
        }
        let image = if let Some(bytes) = bytes {
            tokio::task::spawn_blocking(move || paint::images::decode_image(&bytes))
                .await
                .ok()
                .and_then(Result::ok)
        } else {
            None
        };
        let success = image.is_some();
        // Cancellation must leave a cache miss retryable for the next DOM
        // revision; only completed attempts become negative cache entries.
        requests.attempted.insert(url.clone());
        page.image_taint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(url.clone(), tainted);
        page.image_cache.insert(url.clone(), image);
        drop(requests);
        on_ready(url, success);
    }
}

pub(crate) fn data_image(url: &Url) -> Option<Vec<u8>> {
    let raw = url.as_str().strip_prefix("data:")?;
    let (header, data) = raw.split_once(',')?;
    if !header.split(';').next()?.starts_with("image/") || data.len() > MAX_IMAGE_BYTES * 2 {
        return None;
    }
    let bytes = percent_encoding::percent_decode_str(data).collect::<Vec<_>>();
    let bytes = if header.split(';').any(|s| s.eq_ignore_ascii_case("base64")) {
        base64::engine::general_purpose::STANDARD
            .decode(bytes)
            .ok()?
    } else {
        bytes
    };
    (bytes.len() <= MAX_IMAGE_BYTES).then_some(bytes)
}
