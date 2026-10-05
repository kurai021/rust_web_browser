//! Bounded auxiliary raster/SVG decoding and a 100 MiB decoded-image LRU.
//! No HTML/CSS engine and no external SVG resources or network calls.

use layout::{Images, RasterImage};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use url::Url;

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
pub const CACHE_BYTES: usize = 100 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 4096;

/// Test/demo snapshot encoding uses the same approved auxiliary codec crate.
pub fn encode_png(image: &RasterImage) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &image.rgba,
            image.width,
            image.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
pub fn save_snapshot(
    path: &std::path::Path,
    width: u32,
    height: u32,
    pixels: &[u32],
) -> Result<(), String> {
    if pixels.len() != width as usize * height as usize {
        return Err("snapshot pixel size mismatch".into());
    }
    let rgba = pixels
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255])
        .collect::<Vec<_>>();
    image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())
}

pub fn decode_image(bytes: &[u8]) -> Result<Arc<RasterImage>, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image body limit exceeded".into());
    }
    if bytes[..bytes.len().min(1024)]
        .windows(4)
        .any(|w| w == b"<svg")
    {
        return decode_svg(bytes);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES as u64);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    RasterImage::new(image.width(), image.height(), image.into_raw())
        .ok_or_else(|| "invalid decoded image dimensions".into())
}

fn decode_svg(bytes: &[u8]) -> Result<Arc<RasterImage>, String> {
    if bytes.len() > 2 * 1024 * 1024 || bytes.iter().filter(|&&b| b == b'<').count() > 20_000 {
        return Err("SVG syntax budget exceeded".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let xml = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let mut numbers = 0usize;
    for node in xml.descendants() {
        if node.ancestors().count() > 512 {
            return Err("SVG depth budget exceeded".into());
        }
        for attribute in node.attributes() {
            if !matches!(
                attribute.name(),
                "d" | "points"
                    | "transform"
                    | "viewBox"
                    | "x"
                    | "y"
                    | "x1"
                    | "y1"
                    | "x2"
                    | "y2"
                    | "rx"
                    | "ry"
                    | "r"
                    | "width"
                    | "height"
                    | "stroke-width"
                    | "stdDeviation"
            ) {
                continue;
            }
            bounded_numbers(attribute.value(), &mut numbers)?;
        }
    }
    static FONTS: std::sync::OnceLock<Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    });
    let options = resvg::usvg::Options {
        fontdb: fonts.clone(),
        font_family: "DejaVu Sans".into(),
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_xmltree(&xml, &options).map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size();
    let (width, height) = (size.width(), size.height());
    if width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || width as usize * height as usize * 4 > MAX_DECODED_BYTES
    {
        return Err("SVG raster budget exceeded".into());
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("SVG allocation rejected")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.take();
    // tiny-skia stores premultiplied RGBA; our backends use straight alpha.
    for p in rgba.chunks_exact_mut(4) {
        if p[3] != 0 {
            for c in 0..3 {
                p[c] = (u32::from(p[c]) * 255 / u32::from(p[3])).min(255) as u8;
            }
        }
    }
    RasterImage::new(width, height, rgba).ok_or_else(|| "invalid SVG dimensions".into())
}

// Resource preflight only: SVG grammar/geometry still belongs to usvg/resvg.
// This catches the huge-radius arc expansion found by the Phase 4 fuzzer
// before the auxiliary simplifier allocates millions of cubic segments.
fn bounded_numbers(value: &str, count: &mut usize) -> Result<(), String> {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() && !matches!(bytes[i], b'+' | b'-' | b'.') {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        let mut digits = 0;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits += 1;
        }
        if bytes.get(i) == Some(&b'.') {
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            i = start + 1;
            continue;
        }
        if matches!(bytes.get(i), Some(b'e' | b'E')) {
            let exponent = i;
            i += 1;
            if matches!(bytes.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let first_digit = i;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if first_digit == i {
                i = exponent;
            }
        }
        let number = value.get(start..i).and_then(|s| s.parse::<f64>().ok());
        *count += 1;
        if number.is_none_or(|n| !n.is_finite() || n.abs() > 10_000_000.0) || *count > 100_000 {
            return Err("SVG geometry budget exceeded".into());
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Entry {
    image: Option<Arc<RasterImage>>,
    used: u64,
}
#[derive(Debug, Default)]
struct Cache {
    entries: HashMap<Url, Entry>,
    bytes: usize,
    clock: u64,
}
#[derive(Debug, Default)]
pub struct ImageCache {
    inner: Mutex<Cache>,
}
impl ImageCache {
    /// `Some(None)` caches a failed decode, avoiding repeated requests.
    pub fn get(&self, url: &Url) -> Option<Option<Arc<RasterImage>>> {
        let mut cache = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        cache.clock += 1;
        let used = cache.clock;
        let e = cache.entries.get_mut(url)?;
        e.used = used;
        Some(e.image.clone())
    }
    pub fn insert(&self, url: Url, image: Option<Arc<RasterImage>>) {
        let mut cache = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = cache.entries.remove(&url) {
            cache.bytes -= old.image.map_or(0, |i| i.rgba.len());
        }
        let bytes = image.as_ref().map_or(0, |i| i.rgba.len());
        if bytes > CACHE_BYTES {
            return;
        }
        while cache.bytes + bytes > CACHE_BYTES || cache.entries.len() >= 256 {
            let oldest = cache
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(u, _)| u.clone());
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(old) = cache.entries.remove(&oldest) {
                cache.bytes -= old.image.map_or(0, |i| i.rgba.len());
            }
        }
        cache.clock += 1;
        let used = cache.clock;
        cache.bytes += bytes;
        cache.entries.insert(url, Entry { image, used });
    }
    pub fn snapshot(&self) -> Images {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .iter()
            .filter_map(|(url, e)| Some((url.clone(), e.image.clone()?)))
            .collect()
    }
    pub fn bytes(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }
}
