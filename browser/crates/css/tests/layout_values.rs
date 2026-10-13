use css::{layout_values::*, *};
fn computed(rules: &str) -> std::sync::Arc<ComputedStyle> {
    let url = "https://example.test/".parse().unwrap();
    let d = html::parse_full(b"<div id=x></div>", &url, Default::default());
    let styles = Cascade::default().compute(&d, &[parse_stylesheet(rules, Some(&url))]);
    styles
        .get(d.get_element_by_id("x").unwrap())
        .unwrap()
        .clone()
}
#[test]
fn flex_and_alignment_shorthands_are_atomic_and_cssom_agrees() {
    let s=computed("#x{display:inline-flex;flex:2 3 40%;flex-flow:column-reverse wrap;gap:10px 20px;place-items:center end;inset:1px 2px 3px 4px}");
    assert_eq!(s.layout.mode, Mode::Flex);
    assert!(s.layout.inline && s.layout.column && s.layout.reverse && s.layout.wrap);
    assert_eq!(s.layout.grow, 2.0);
    assert_eq!(s.layout.shrink, 3.0);
    assert_eq!(s.get_property_value("flex-basis"), "40%");
    assert_eq!(s.layout.align_items, Align::Center);
    assert_eq!(s.layout.justify_items, Align::End);
    assert_eq!(s.get_property_value("left"), "4px");
    let s = computed("#x{flex:2 3 20px;flex:4 garbage;gap:10px;gap:-1px 20px}");
    assert_eq!(s.layout.grow, 2.0);
    assert_eq!(s.get_property_value("row-gap"), "10px");
}
#[test]
fn bounded_tracks_rectangular_areas_and_placements() {
    let s=computed("#x{display:grid;grid-template-columns:repeat(2,minmax(10px,1fr)) 20%;grid-template-areas:'a a .' 'b c c';grid-area:2/1/span 2/span 3}");
    assert_eq!(s.layout.columns.values.len(), 3);
    assert_eq!(s.layout.areas.len(), 2);
    assert_eq!(s.layout.placement[2], GridLine::Span(2));
    assert_eq!(s.layout.placement[3], GridLine::Span(3));
    assert!(!style::supported_property(
        "grid-template-columns",
        &token::tokenize_all("repeat(1000000,1fr)")
    ));
    assert!(!style::supported_property(
        "grid-template-areas",
        &token::tokenize_all("'a a' 'a b'")
    ));
    assert!(!style::supported_property(
        "flex-shrink",
        &token::tokenize_all("-1")
    ));
}
