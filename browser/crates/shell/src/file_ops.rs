//! Explicit user-initiated import/export/print helpers, run on a background worker.
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub fn escape(source: &str) -> String {
    source
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn import_bookmarks(bytes: &[u8]) -> Result<Vec<profile::Bookmark>, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Bookmark import exceeds 8 MiB".into());
    }
    let base = url::Url::parse("about:bookmarks").map_err(|e| e.to_string())?;
    let doc = html::parse_full(
        bytes,
        &base,
        html::ParseOpts {
            max_nodes: 30_000,
            max_depth: 64,
            max_attr_len: 8192,
            scripting_enabled: false,
        },
    );
    let mut seen = std::collections::HashSet::new();
    Ok(doc
        .get_elements_by_tag_name("a")
        .into_iter()
        .filter_map(|node| {
            let url = profile::clean_url(doc.get_attribute(node, "href")?)?;
            if !seen.insert(url.clone()) {
                return None;
            }
            Some(profile::Bookmark {
                url,
                title: profile::short_title(&doc.text_content(node)),
                added: profile::now(),
            })
        })
        .take(profile::MAX_BOOKMARKS)
        .collect())
}
pub fn export_bookmarks(bookmarks: &[profile::Bookmark]) -> String {
    let mut html = "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n".to_owned();
    for b in bookmarks.iter().take(profile::MAX_BOOKMARKS) {
        html.push_str(&format!(
            "<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>\n",
            escape(b.url.as_str()),
            b.added,
            escape(&b.title)
        ));
    }
    html.push_str("</DL><p>\n");
    html
}
pub fn read_bookmarks(path: &Path) -> Result<Vec<profile::Bookmark>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    import_bookmarks(&bytes)
}
/// Exports never overwrite an existing file. Duplicate names receive a suffix.
pub fn save_export(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Choose a directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid export filename")?;
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map_or_else(String::new, |s| format!(".{s}"));
    for i in 0..10_000 {
        let path = parent.join(if i == 0 {
            name.to_owned()
        } else {
            format!("{stem} ({i}){ext}")
        });
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(error.to_string());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Too many duplicate export filenames".into())
}
/// Basic A4 raster PDF from the owned display list; at most one page bitmap is
/// resident at a time. No OS dialog, network or external renderer is involved.
pub fn print_pdf(scene: &paint::Scene, size: layout::Size) -> Result<Vec<u8>, String> {
    use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};
    if !size.width.is_finite() || !size.height.is_finite() || size.width <= 0.0 || size.height < 0.0
    {
        return Err("Invalid print dimensions".into());
    }
    let width = size.width.clamp(1.0, 2400.0).ceil() as u32;
    let scale = width as f32 / size.width;
    let height = (width as f32 * 842.0 / 595.0).ceil() as u32;
    let count = ((size.height * scale).max(1.0) / height as f32).ceil() as usize;
    if count > 50 {
        return Err("Basic print is limited to 50 pages; use a smaller document".into());
    }
    let mut pdf = Pdf::new();
    let tree = Ref::new(2);
    pdf.catalog(Ref::new(1)).pages(tree);
    let page_ids: Vec<_> = (0..count).map(|i| Ref::new(3 + i as i32 * 3)).collect();
    pdf.pages(tree)
        .kids(page_ids.iter().copied())
        .count(count as i32);
    let mut compressed_bytes = 0usize;
    for (i, &page_id) in page_ids.iter().enumerate() {
        let image_id = Ref::new(4 + i as i32 * 3);
        let content_id = Ref::new(5 + i as i32 * 3);
        let mut pixels = vec![0xffffff; width as usize * height as usize];
        scene.paint_software(
            &mut pixels,
            width,
            height,
            layout::Rect::new(0.0, 0.0, width as f32, height as f32),
            (0.0, -(i as f32 * height as f32)),
            scale,
            &Default::default(),
        );
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        for row in pixels.chunks(width as usize) {
            let rgb: Vec<u8> = row
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            encoder.write_all(&rgb).map_err(|e| e.to_string())?;
        }
        let compressed = encoder.finish().map_err(|e| e.to_string())?;
        compressed_bytes = compressed_bytes.saturating_add(compressed.len());
        if compressed_bytes > 64 * 1024 * 1024 {
            return Err("PDF exceeds 64 MiB output limit".into());
        }
        let mut page = pdf.page(page_id);
        page.parent(tree)
            .media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
            .contents(content_id);
        page.resources().x_objects().pair(Name(b"Page"), image_id);
        page.finish();
        let mut image = pdf.image_xobject(image_id, &compressed);
        image
            .width(width as i32)
            .height(height as i32)
            .bits_per_component(8)
            .filter(pdf_writer::Filter::FlateDecode);
        image.color_space().device_rgb();
        image.finish();
        let mut content = Content::new();
        content
            .save_state()
            .transform([595.0, 0.0, 0.0, 842.0, 0.0, 0.0])
            .x_object(Name(b"Page"))
            .restore_state();
        pdf.stream(content_id, &content.finish());
    }
    let bytes = pdf.finish();
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("PDF exceeds 64 MiB output limit".into());
    }
    Ok(bytes)
}
