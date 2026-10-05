//! Meaningful malformed-input/resource-boundary checks, not just roundtrip
//! tests. Coverage-guided equivalents are in css/fuzz/.
use css::{Cascade, Stylesheet};

#[test]
fn root_rem_units_and_pseudo_styles_resolve_without_applying_print_sheets() {
    let url = "https://example.com/".parse().unwrap();
    let doc = html::parse_full(b"<p id=target>text</p>", &url, html::ParseOpts::default());
    let sheet=css::parse_stylesheet("html{font-size:2rem;margin-left:1rem} ::before{content:'prefix';color:green} @media print{p{display:none}}",None);
    let styles = Cascade::default().compute(&doc, &[sheet]);
    let root = styles.get(doc.document_element().unwrap()).unwrap();
    assert_eq!(root.font_size, 32.0);
    assert_eq!(root.get_property_value("margin-left"), "32px");
    let target = doc.get_element_by_id("target").unwrap();
    assert_ne!(
        styles.get(target).unwrap().display,
        css::style::Display::None
    );
    assert_eq!(
        styles
            .pseudo(target, css::selectors::PseudoElement::Before)
            .unwrap()
            .content,
        "prefix"
    );
}

#[test]
fn malformed_css_has_bounded_recovery_and_keeps_later_rules() {
    let url = "https://example.com/".parse().unwrap();
    let doc = html::parse_full(b"<p id='target'>Text</p>", &url, html::ParseOpts::default());
    let target = doc.get_element_by_id("target").unwrap();
    let seeds = [
        "p{color: red} /* ",
        "@media ((( ",
        "#\u{FFFD} {color:blue}",
        "p{--a:var(--a);color:var(--a,green)}",
        "p{color:rgb(1 2 3 / 50%)}",
        "p{width:calc(10px / 0)}",
        "@unknown { p {color:red} }",
        "p::before{content:'x';color:purple}",
    ];
    let mut state = 0xabc123_u64;
    for index in 0..1000 {
        let mut bytes = seeds[index % seeds.len()].as_bytes().to_vec();
        for _ in 0..8 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let byte = state as u8;
            let at = (state as usize) % bytes.len();
            bytes[at] = byte;
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        text.push_str("\n} p {color:green}");
        let sheet = css::parse_stylesheet(&text, Some(&url));
        assert!(sheet.errors.len() <= 100);
        let styles = Cascade::default().compute(&doc, &[sheet]);
        assert!(styles.get(target).unwrap().font_size.is_finite());
        let _ = css::syntax::parse_declarations(&text);
        let _ = css::selectors::parse_selector_list(&text);
    }
    let oversized = " ".repeat(css::syntax::MAX_STYLESHEET_BYTES + 1);
    let sheet = css::parse_stylesheet(&oversized, None);
    assert!(sheet.rules.is_empty());
    assert!(!sheet.errors.is_empty());
    let bad = css::parse_stylesheet(
        "@unknown {p{color:red}} p{color:green; font-size:nonsense}",
        None,
    );
    assert_eq!(
        Cascade::default()
            .compute(&doc, &[bad])
            .get(target)
            .unwrap()
            .get_property_value("color"),
        "rgb(0, 128, 0)"
    );
}

#[test]
fn invalid_values_do_not_win_and_pending_variables_use_initial_or_inheritance() {
    let url = "https://example.com/".parse().unwrap();
    let doc = html::parse_full(
        b"<div style='color:blue'><p id='target'>Text</p></div>",
        &url,
        html::ParseOpts::default(),
    );
    let target = doc.get_element_by_id("target").unwrap();
    let css="p{color:red;color:nonsense; margin:10px 20px;padding:1px -2px} p{color:var(--missing);margin-top:var(--missing);line-height:1.5}";
    let sheet = css::parse_stylesheet(css, None);
    let styles = Cascade::default().compute(&doc, &[sheet]);
    let style = styles.get(target).unwrap();
    assert_eq!(style.get_property_value("color"), "rgb(0, 0, 255)");
    assert_eq!(style.get_property_value("margin-top"), "0px");
    assert_eq!(style.get_property_value("margin-right"), "20px");
    assert_eq!(style.get_property_value("padding-right"), "0px");
    assert_eq!(style.line_height.pixels(style.font_size), 24.0);
    assert!(Cascade::default()
        .compute(&doc, &[Stylesheet::default()])
        .get(target)
        .is_some());
}
