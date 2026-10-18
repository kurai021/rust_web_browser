pub mod support;
#[test]
fn find_uses_visible_unicode_shaped_text_and_pdf_is_paginated_from_the_owned_scene() {
    let url: url::Url = "http://127.0.0.1/".parse().unwrap();
    let head = net::ResponseHead {
        url: url.clone(),
        status: 200,
        content_type: Some("text/html".into()),
        script_policies: Vec::new(),
    };
    let doc=html::parse_full(b"<style>.hidden{display:none}</style><p>caf&#233; and CAF&#201;</p><p>blue <b>fox</b></p><p class=hidden>not painted</p>",&url,Default::default());
    let page = shell::page::Page::preview(&head, doc);
    let styles = page.computed_styles(Default::default());
    let mut fonts = cosmic_text::FontSystem::new();
    let (layout, scene) = shell::viewport::layout_page(
        &page,
        &styles,
        layout::Size::new(400.0, 300.0),
        &mut fonts,
        &mut paint::Rasterizer::default(),
    );
    let matches = shell::find::search(&layout, "CAFÉ");
    assert_eq!(matches.len(), 2);
    assert_eq!(shell::find::search(&layout, "cafe\u{301}").len(), 2);
    assert!(matches.iter().all(|m| m
        .parts
        .iter()
        .all(|p| p.rect.width > 0.0 && p.rect.height > 0.0)));
    assert!(shell::find::search(&layout, "not painted").is_empty());
    assert_eq!(shell::find::search(&layout, "blue fox").len(), 1);
    let pdf = shell::file_ops::print_pdf(&scene, layout::Size::new(400.0, 2000.0)).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(pdf.windows(8).any(|w| w == b"/Count 4"));
}
#[test]
fn bookmark_import_is_inert_and_exports_escape_html_without_overwriting_user_files() {
    let imported=shell::file_ops::import_bookmarks(b"<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><DT><A HREF='https://example.test/a?x=1&amp;y=2'>A &amp; B</A><script>throw 1;</script><A HREF='javascript:alert(1)'>ignored</A></DL>").unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].title, "A & B");
    let html = shell::file_ops::export_bookmarks(&imported);
    let roundtrip = shell::file_ops::import_bookmarks(html.as_bytes()).unwrap();
    assert_eq!(roundtrip[0].url, imported[0].url);
    assert_eq!(roundtrip[0].title, imported[0].title);
    let directory = support::Directory::new();
    let path = directory.0.join("bookmarks.html");
    std::fs::write(&path, b"existing user file").unwrap();
    let saved = shell::file_ops::save_export(&path, html.as_bytes()).unwrap();
    assert_ne!(saved, path);
    assert_eq!(std::fs::read(path).unwrap(), b"existing user file");
}
