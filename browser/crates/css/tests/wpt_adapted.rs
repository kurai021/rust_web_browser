//! Actual WPT inputs/assertions adapted to Rust APIs, without JS execution.
//! See wpt/README.md for provenance and the exact subtest denominator.

use css::selectors::{parse_selector_list, MatchContext};
use css::{Cascade, ComputedStyles, Stylesheet};
use html::Document;

fn fixture(html: &str) -> (Document, Vec<Stylesheet>) {
    let url = "https://example.com/".parse().unwrap();
    let doc = html::parse_full(html.as_bytes(), &url, html::ParseOpts::default());
    let sheets = doc
        .get_elements_by_tag_name("style")
        .into_iter()
        .map(|id| css::parse_stylesheet(&doc.text_content(id), Some(&url)))
        .collect();
    (doc, sheets)
}
fn computed(html: &str) -> (Document, ComputedStyles) {
    let (doc, sheets) = fixture(html);
    let styles = Cascade::default().compute(&doc, &sheets);
    (doc, styles)
}

#[test]
fn syntax_ident_three_code_points_eight_assertions() {
    let (doc, styles) = computed(include_str!("wpt/ident-three-code-points.html"));
    let items = doc.get_elements_by_class_name("item");
    assert_eq!(items.len(), 8);
    for item in items {
        assert_eq!(
            styles
                .get(item)
                .unwrap()
                .get_property_value("background-color"),
            "rgb(0, 128, 0)"
        );
    }
}

#[test]
fn selectors_not_specificity_eight_assertions() {
    let (doc, styles) = computed(include_str!("wpt/not-specificity.html"));
    let target = styles.get(doc.get_element_by_id("div").unwrap()).unwrap();
    for index in 0..8 {
        assert_eq!(target.get_property_value(&format!("--t{index}")), "PASS");
    }
}

#[test]
fn cascade_important_vs_inline_eight_assertions() {
    // Each static document represents one of the upstream JS assignments.
    // Input CSS and expected computed values are unchanged.
    for inline in ["", "opacity:0.75", "opacity:1", ""] {
        let source = include_str!("wpt/important-vs-inline-001.html")
            .replace("id=\"el\"", &format!("id=\"el\" style=\"{inline}\""));
        let (doc, styles) = computed(&source);
        assert_eq!(
            styles
                .get(doc.get_element_by_id("el").unwrap())
                .unwrap()
                .get_property_value("opacity"),
            "0.5"
        );
    }
    for inline in ["", "font-size:24px", "font-size:36px", ""] {
        let source = include_str!("wpt/important-vs-inline-002.html")
            .replace("id=\"el\"", &format!("id=\"el\" style=\"{inline}\""));
        let (doc, styles) = computed(&source);
        assert_eq!(
            styles
                .get(doc.get_element_by_id("el").unwrap())
                .unwrap()
                .get_property_value("line-height"),
            "36px"
        );
    }
}

#[test]
fn cascade_root_inherit_initial_four_assertions() {
    let (doc, styles) = computed(include_str!("wpt/inherit-initial.html"));
    let root = styles.get(doc.document_element().unwrap()).unwrap();
    for (property, expected) in [
        ("z-index", "auto"),
        ("position", "static"),
        ("overflow", "visible"),
        ("background-color", "rgba(0, 0, 0, 0)"),
    ] {
        assert_eq!(root.get_property_value(property), expected);
    }
}

#[test]
fn selectors_not_complex_twenty_assertions() {
    // css/selectors/not-complex.html: exact markup, selectors and expected IDs.
    let (doc,_)=fixture("<main id=main><div id=a><div id=d></div></div><div id=b><div id=e></div></div><div id=c><div id=f></div></div></main>");
    let tests = [
        (":not(#a)", "b,c,d,e,f"),
        (":not(#a #d)", "a,b,c,e,f"),
        (":not(#b div)", "a,b,c,d,f"),
        (":not(div div)", "a,b,c"),
        (":not(div + div)", "a,d,e,f"),
        (":not(main > div)", "d,e,f"),
        (":not(#a, #b)", "c,d,e,f"),
        (":not(#f, main > div)", "d,e"),
        (":not(div + div + div, div + div > div)", "a,b,d"),
        (":not(div:nth-child(1))", "b,c"),
        (":not(:not(div))", "a,b,c,d,e,f"),
        (":not(:not(:not(div)))", ""),
        (":not(div, span)", ""),
        (":not(span, p)", "a,b,c,d,e,f"),
        (":not(#unknown, .unknown)", "a,b,c,d,e,f"),
        (":not(#unknown > div, span)", "a,b,c,d,e,f"),
        (":not(#unknown ~ div, span)", "a,b,c,d,e,f"),
        (":not(:hover div)", "a,b,c,d,e,f"),
        (":not(:link div)", "a,b,c,d,e,f"),
        (":not(:visited div)", "a,b,c,d,e,f"),
    ];
    for (text, expected) in tests {
        let selectors = parse_selector_list(text).unwrap();
        let mut matched = Vec::new();
        for name in ["a", "b", "c", "d", "e", "f"] {
            if selectors.iter().any(|s| {
                s.matches(
                    &doc,
                    doc.get_element_by_id(name).unwrap(),
                    MatchContext::default(),
                )
            }) {
                matched.push(name);
            }
        }
        assert_eq!(matched.join(","), expected, "{text}");
    }
}

#[test]
fn selectors_escaped_attribute_space_one_assertion() {
    // css/selectors/attribute-selector-escape.html, a jsdom regression.
    let spaces = " ".repeat(30);
    let html = format!("<div id='div' style='{spaces}'>hello</div>");
    let (doc, _) = fixture(&html);
    let selector = format!("[style={}]", "\\ ".repeat(30));
    assert!(parse_selector_list(&selector).unwrap()[0].matches(
        &doc,
        doc.get_element_by_id("div").unwrap(),
        MatchContext::default()
    ));
}

#[test]
fn selectors_nth_of_list_three_assertions() {
    // Selected Success cases from nth-child-and-nth-last-child.html.
    for (target, selector) in [
        ("target1", "target1"),
        ("target class='target'", ".target"),
        ("target data-target", "[data-target]"),
    ] {
        let tag = target.split_ascii_whitespace().next().unwrap();
        let html = format!(
            "<div><div></div><div></div><{target} id='success'>Success</{tag}><div></div></div>"
        );
        let (doc, _) = fixture(&html);
        let query = format!(":nth-child(1 of {selector}):nth-last-child(1 of {selector})");
        assert!(parse_selector_list(&query).unwrap()[0].matches(
            &doc,
            doc.get_element_by_id("success").unwrap(),
            MatchContext::default()
        ));
    }
}
