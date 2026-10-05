#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(16 * 1024)];
    let text = String::from_utf8_lossy(data);
    let url = url::Url::parse("https://example.com/").expect("static URL");
    let sheet = css::parse_stylesheet(&text, Some(&url));
    let doc=html::parse_full(b"<!DOCTYPE html><body><div id='target' class='a'><p>one</p><p class='a'>two</p><a href='/'>link</a></div></body>",&url,html::ParseOpts::default());
    let _ = css::syntax::parse_declarations(&text);
    if let Ok(selectors) = css::selectors::parse_selector_list(&text) {
        for selector in selectors {
            let _ = selector.matches(&doc, doc.body().unwrap_or(0), Default::default());
        }
    }
    let _ = css::Cascade::default().compute(&doc, &[sheet]);
});
