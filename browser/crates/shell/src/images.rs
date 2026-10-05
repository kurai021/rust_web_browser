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
        requests.attempted.insert(url.clone());
        let cap = MAX_IMAGE_BYTES.min(MAX_PAGE_IMAGE_BYTES - requests.bytes);
        let bytes = if url.scheme() == "data" {
            data_image(&url).filter(|b| b.len() <= cap)
        } else if allowed_subresource(&page.url, &url) {
            match client.fetch_limited(&url, cap, &mut |_| {}).await {
                Ok(fetched) if allowed_subresource(&page.url, &fetched.url) => Some(fetched.bytes),
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
        page.image_cache.insert(url.clone(), image);
        drop(requests);
        on_ready(url, success);
    }
}

fn data_image(url: &Url) -> Option<Vec<u8>> {
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
