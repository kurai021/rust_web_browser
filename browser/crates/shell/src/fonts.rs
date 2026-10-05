//! Page-owned @font-face sources. Decode off the UI thread; the UI keeps
//! rendering fallback text while the assets load (font-display: swap).

use crate::page::{allowed_subresource, Page};
use base64::Engine;
use css::stylesheet::{FontFace, FontSource};
use css::{Cascade, Environment};
use net::Client;
use std::io::Read;
use std::sync::Arc;
use url::Url;

pub const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGE_FONTS: usize = 16;
const MAX_FONT_REQUESTS: usize = 64;

#[derive(Debug, Clone)]
pub struct FontAsset {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub data: Arc<Vec<u8>>,
}

#[derive(Debug, Default)]
pub(crate) struct FontCache {
    entries: tokio::sync::Mutex<std::collections::BTreeMap<String, Option<FontAsset>>>,
    requests: std::sync::atomic::AtomicUsize,
}

pub async fn load_fonts(client: &Client, page: &Page, env: Environment) -> Vec<FontAsset> {
    let cascade = Cascade {
        environment: env,
        ..Cascade::default()
    };
    let styles = cascade.compute(&page.document, &page.stylesheets);
    let mut needed = std::collections::BTreeSet::new();
    let mut nodes = vec![page.document.root()];
    while let Some(id) = nodes.pop() {
        if let Some(style) = styles.get(id) {
            if style.display == css::style::Display::None {
                continue;
            }
            {
                for name in &style.font_family {
                    needed.insert(name.to_ascii_lowercase());
                }
            }
        }
        nodes.extend(page.document.get(id).children.iter().copied());
    }
    let mut assets = Vec::new();
    for face in cascade
        .font_faces(&page.stylesheets)
        .into_iter()
        .filter(|f| needed.contains(&f.family.to_ascii_lowercase()))
        .take(MAX_PAGE_FONTS)
    {
        let key = format!(
            "{}:{}:{}:{:?}",
            face.family, face.weight, face.italic, face.sources
        );
        // Serialize page-local cache misses, preventing duplicate requests
        // when navigation and the first resize overlap. UI never awaits it.
        let mut cache = page.font_cache.entries.lock().await;
        let cached = cache.get(&key).cloned();
        if let Some(asset) = cached {
            if let Some(asset) = asset {
                assets.push(asset);
            }
            continue;
        }
        let asset = load_face(client, &page.url, &face, &page.font_cache.requests).await;
        cache.insert(key, asset.clone());
        if let Some(asset) = asset {
            assets.push(asset);
        }
    }
    assets
}

async fn load_face(
    client: &Client,
    document_url: &Url,
    face: &FontFace,
    requests: &std::sync::atomic::AtomicUsize,
) -> Option<FontAsset> {
    let mut local_fonts = None;
    for source in face.sources.iter().take(16) {
        if let FontSource::Local(name) = source {
            let db = local_fonts.get_or_insert_with(|| {
                let mut db = cosmic_text::fontdb::Database::new();
                db.load_system_fonts();
                db
            });
            let id = db
                .faces()
                .find(|f| {
                    f.families
                        .iter()
                        .any(|(family, _)| family.eq_ignore_ascii_case(name))
                })
                .map(|f| f.id);
            if let Some(data) = id
                .and_then(|id| {
                    db.with_face_data(id, |data, _| {
                        (data.len() <= MAX_FONT_BYTES).then(|| data.to_vec())
                    })
                })
                .flatten()
            {
                return Some(FontAsset {
                    family: face.family.clone(),
                    weight: face.weight,
                    italic: face.italic,
                    data: Arc::new(data),
                });
            }
            continue;
        }
        let FontSource::Url { url, .. } = source else {
            continue;
        };
        let bytes = if url.scheme() == "data" {
            match data_font(url) {
                Some(bytes) => bytes,
                None => continue,
            }
        } else {
            if !allowed_subresource(document_url, url) {
                continue;
            }
            if requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= MAX_FONT_REQUESTS {
                return None;
            }
            match client.fetch_limited(url, MAX_FONT_BYTES, &mut |_| {}).await {
                Ok(fetched) if allowed_subresource(document_url, &fetched.url) => fetched.bytes,
                _ => continue,
            }
        };
        if let Some(data) = decode_font(&bytes) {
            return Some(FontAsset {
                family: face.family.clone(),
                weight: face.weight,
                italic: face.italic,
                data: Arc::new(data),
            });
        }
    }
    None
}

fn data_font(url: &Url) -> Option<Vec<u8>> {
    if url.as_str().len() > 2 * 1024 * 1024 {
        return None;
    }
    let (header, raw) = url.as_str().strip_prefix("data:")?.split_once(',')?;
    let mime = header.split(';').next()?.to_ascii_lowercase();
    if !matches!(
        mime.as_str(),
        "font/ttf"
            | "font/otf"
            | "font/woff"
            | "font/woff2"
            | "application/font-woff"
            | "application/octet-stream"
    ) {
        return None;
    }
    let bytes = percent_encoding::percent_decode_str(raw).collect::<Vec<u8>>();
    if header.split(';').any(|s| s.eq_ignore_ascii_case("base64")) {
        base64::engine::general_purpose::STANDARD
            .decode(&bytes)
            .ok()
    } else {
        Some(bytes)
    }
}

pub fn decode_font(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > MAX_FONT_BYTES || bytes.len() < 12 {
        return None;
    }
    match bytes.get(..4)? {
        b"\0\x01\0\0" | b"OTTO" | b"true" => Some(bytes.to_vec()),
        b"wOFF" => decode_woff(bytes),
        b"wOF2" => {
            if bytes.len() < 48
                || be32(bytes, 8)? as usize != bytes.len()
                || !(1..=128).contains(&be16(bytes, 12)?)
                || be32(bytes, 16)? as usize > MAX_FONT_BYTES
            {
                return None;
            }
            let bounded = bounded_woff2(bytes)?;
            // An isolated decoder boundary: malformed remote fonts never
            // propagate a dependency panic into navigation.
            let decoded =
                std::panic::catch_unwind(|| woff2_patched::convert_woff2_to_ttf(&mut &bounded[..]))
                    .ok()?
                    .ok()?;
            if decoded.len() > MAX_FONT_BYTES {
                None
            } else {
                Some(decoded)
            }
        }
        _ => None,
    }
}

/// Validate directory totals and cap Brotli expansion before invoking a
/// decoder whose internal Vec writer is not resource-bounded. Only SFNT
/// single-font WOFF2 files are accepted in this path (not TTC collections).
fn bounded_woff2(bytes: &[u8]) -> Option<Vec<u8>> {
    if !matches!(bytes.get(4..8)?, b"\0\x01\0\0" | b"OTTO") {
        return None;
    }
    let mut cursor = 48;
    let mut expanded = 0usize;
    let mut original = 12 + 16 * be16(bytes, 12)? as usize;
    for _ in 0..be16(bytes, 12)? {
        let flags = *bytes.get(cursor)?;
        cursor += 1;
        let index = flags & 63;
        let glyf_loca = if index == 63 {
            let tag = bytes.get(cursor..cursor + 4)?;
            cursor += 4;
            tag == b"glyf" || tag == b"loca"
        } else {
            index == 10 || index == 11
        };
        let length = base128(bytes, &mut cursor)?;
        original = original.checked_add(align4(length))?;
        let transformed = if glyf_loca {
            flags >> 6 != 3
        } else {
            flags >> 6 != 0
        };
        let table_size = if transformed {
            base128(bytes, &mut cursor)?
        } else {
            length
        };
        expanded = expanded.checked_add(table_size)?;
        if expanded > MAX_FONT_BYTES || original > MAX_FONT_BYTES {
            return None;
        }
    }
    let compressed = be32(bytes, 20)? as usize;
    let end = cursor.checked_add(compressed)?;
    let stream = bytes.get(cursor..end)?;
    let mut output = Vec::new();
    brotli::Decompressor::new(stream, 4096)
        .take(expanded as u64 + 1)
        .read_to_end(&mut output)
        .ok()?;
    if output.len() != expanded {
        return None;
    }
    // Exclude metadata/private blocks from the decoder's Brotli reader.
    let mut bounded = bytes.get(..end)?.to_vec();
    bounded[8..12].copy_from_slice(&(end as u32).to_be_bytes());
    bounded[28..48].fill(0);
    Some(bounded)
}

fn base128(bytes: &[u8], cursor: &mut usize) -> Option<usize> {
    let mut value = 0u32;
    for index in 0..5 {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        if (index == 0 && byte == 0x80) || value & 0xfe00_0000 != 0 {
            return None;
        }
        value = (value << 7) | (byte & 127) as u32;
        if byte & 128 == 0 {
            return Some(value as usize);
        }
    }
    None
}
fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
fn align4(n: usize) -> usize {
    (n + 3) & !3
}
fn decode_woff(bytes: &[u8]) -> Option<Vec<u8>> {
    let tables = be16(bytes, 12)? as usize;
    let total = be32(bytes, 16)? as usize;
    if bytes.len() < 44
        || be32(bytes, 8)? as usize != bytes.len()
        || !(1..=128).contains(&tables)
        || total > MAX_FONT_BYTES
    {
        return None;
    }
    let mut output = vec![0u8; 12 + 16 * tables];
    output[..4].copy_from_slice(bytes.get(4..8)?);
    output[4..6].copy_from_slice(&(tables as u16).to_be_bytes());
    let power = 1usize << (usize::BITS - 1 - tables.leading_zeros());
    output[6..8].copy_from_slice(&((power * 16) as u16).to_be_bytes());
    output[8..10].copy_from_slice(&(power.trailing_zeros() as u16).to_be_bytes());
    output[10..12].copy_from_slice(&((tables * 16 - power * 16) as u16).to_be_bytes());
    for index in 0..tables {
        let record = 44 + index * 20;
        let offset = be32(bytes, record + 4)? as usize;
        let compressed = be32(bytes, record + 8)? as usize;
        let original = be32(bytes, record + 12)? as usize;
        if original > MAX_FONT_BYTES
            || compressed > original
            || output.len().saturating_add(align4(original)) > MAX_FONT_BYTES
        {
            return None;
        }
        let input = bytes.get(offset..offset.checked_add(compressed)?)?;
        let data = if compressed == original {
            input.to_vec()
        } else {
            let mut data = Vec::new();
            flate2::read::ZlibDecoder::new(input)
                .take(original as u64 + 1)
                .read_to_end(&mut data)
                .ok()?;
            data
        };
        if data.len() != original {
            return None;
        }
        let target = 12 + 16 * index;
        let table_offset = output.len();
        output[target..target + 4].copy_from_slice(bytes.get(record..record + 4)?);
        output[target + 4..target + 8].copy_from_slice(bytes.get(record + 16..record + 20)?);
        output[target + 8..target + 12].copy_from_slice(&(table_offset as u32).to_be_bytes());
        output[target + 12..target + 16].copy_from_slice(&(original as u32).to_be_bytes());
        output.extend(data);
        output.resize(align4(output.len()), 0);
    }
    (output.len() == total).then_some(output)
}

/// Register CSS aliases/descriptors in a page-owned font database.
/// Returns the added IDs so the shell can remove them on navigation.
pub fn install_fonts(
    system: &mut cosmic_text::FontSystem,
    assets: &[FontAsset],
) -> Vec<cosmic_text::fontdb::ID> {
    use cosmic_text::fontdb::{Source, Style, Weight};
    let mut added = Vec::new();
    for asset in assets {
        let alias = web_family(&asset.family);
        if system.db().faces().any(|face| {
            face.families.iter().any(|(family, _)| family == &alias)
                && face.weight.0 == asset.weight
                && (face.style == Style::Italic) == asset.italic
        }) {
            continue;
        }
        let before = system
            .db()
            .faces()
            .map(|f| f.id)
            .collect::<std::collections::HashSet<_>>();
        let ids = system
            .db_mut()
            .load_font_source(Source::Binary(asset.data.clone()));
        for id in ids {
            if let Some(mut face) = system.db().face(id).cloned() {
                system.db_mut().remove_face(id);
                for (name, _) in &mut face.families {
                    *name = alias.clone();
                }
                face.weight = Weight(asset.weight);
                face.style = if asset.italic {
                    Style::Italic
                } else {
                    Style::Normal
                };
                system.db_mut().push_face_info(face);
            }
        }
        added.extend(
            system
                .db()
                .faces()
                .filter(|face| !before.contains(&face.id))
                .map(|f| f.id),
        );
    }
    added
}

pub(crate) fn web_family(family: &str) -> String {
    format!("__browser_web_{}", family.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_bounds_and_mime_do_not_accept_images() {
        assert!(decode_font(b"<html>not a font</html>").is_none());
        assert!(data_font(&"data:image/png;base64,AAAA".parse().unwrap()).is_none());
        let mut woff = vec![0u8; 48];
        woff[..4].copy_from_slice(b"wOF2");
        woff[8..12].copy_from_slice(&48u32.to_be_bytes());
        woff[12..14].copy_from_slice(&1u16.to_be_bytes());
        woff[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_font(&woff).is_none());
    }

    #[test]
    fn woff1_tables_decompress_into_a_usable_sfnt() {
        use std::io::Write;
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_system_fonts();
        let sfnt = db
            .faces()
            .find_map(|face| {
                db.with_face_data(face.id, |bytes, _| {
                    (bytes.len() < MAX_FONT_BYTES
                        && matches!(bytes.get(..4), Some(b"\0\x01\0\0" | b"OTTO")))
                    .then(|| bytes.to_vec())
                })
                .flatten()
            })
            .expect("test requires an installed TTF/OTF font (fonts-dejavu-core)");
        let count = be16(&sfnt, 4).unwrap() as usize;
        let mut encoded = vec![0u8; 44 + 20 * count];
        encoded[..4].copy_from_slice(b"wOFF");
        encoded[4..8].copy_from_slice(&sfnt[..4]);
        encoded[12..14].copy_from_slice(&(count as u16).to_be_bytes());
        let mut reconstructed_size = 12 + 16 * count;
        for index in 0..count {
            let at = 12 + 16 * index;
            let offset = be32(&sfnt, at + 8).unwrap() as usize;
            let length = be32(&sfnt, at + 12).unwrap() as usize;
            let data = &sfnt[offset..offset + length];
            let mut compressor =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            compressor.write_all(data).unwrap();
            let compressed = compressor.finish().unwrap();
            let packed = if compressed.len() < data.len() {
                compressed
            } else {
                data.to_vec()
            };
            let record = 44 + 20 * index;
            let packed_offset = encoded.len();
            encoded[record..record + 4].copy_from_slice(&sfnt[at..at + 4]);
            encoded[record + 4..record + 8].copy_from_slice(&(packed_offset as u32).to_be_bytes());
            encoded[record + 8..record + 12].copy_from_slice(&(packed.len() as u32).to_be_bytes());
            encoded[record + 12..record + 16].copy_from_slice(&(length as u32).to_be_bytes());
            encoded[record + 16..record + 20].copy_from_slice(&sfnt[at + 4..at + 8]);
            encoded.extend(packed);
            encoded.resize(align4(encoded.len()), 0);
            reconstructed_size += align4(length);
        }
        let encoded_length = encoded.len();
        encoded[8..12].copy_from_slice(&(encoded_length as u32).to_be_bytes());
        encoded[16..20].copy_from_slice(&(reconstructed_size as u32).to_be_bytes());
        let decoded = decode_font(&encoded).expect("WOFF1 must decode");
        assert_eq!(decoded.len(), reconstructed_size);
        let mut result = cosmic_text::fontdb::Database::new();
        result.load_font_data(decoded);
        assert!(result.faces().next().is_some());
    }

    #[test]
    #[ignore = "optional real WOFF2 fixture; set BROWSER_WOFF2_FIXTURE"]
    fn real_woff2_fixture_decodes_and_registers_css_alias() {
        let path = std::env::var("BROWSER_WOFF2_FIXTURE").expect("fixture path");
        let bytes = std::fs::read(path).unwrap();
        let decoded = decode_font(&bytes).expect("real WOFF2 must decode");
        let asset = FontAsset {
            family: "WebFontTest".into(),
            weight: 400,
            italic: false,
            data: Arc::new(decoded),
        };
        let mut system = cosmic_text::FontSystem::new();
        let ids = install_fonts(&mut system, &[asset]);
        assert!(!ids.is_empty());
        assert!(system.db().faces().any(|f| f
            .families
            .iter()
            .any(|(family, _)| family == &web_family("WebFontTest"))));
    }
}
