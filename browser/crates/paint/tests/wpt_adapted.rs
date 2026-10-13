//! WPT reftest inputs reduced to their geometric bodies; compare the reference.
use cosmic_text::FontSystem;
use layout::*;
use paint::*;
fn green(source: &[u8], expected_left: f32) {
    let url = "https://example.test/".parse().unwrap();
    let doc = html::parse_full(source, &url, Default::default());
    let sheets = doc
        .get_elements_by_tag_name("style")
        .into_iter()
        .map(|i| css::parse_stylesheet(&doc.text_content(i), Some(&url)))
        .collect::<Vec<_>>();
    let styles = css::Cascade::default().compute(&doc, &sheets);
    let mut tree = build_render_tree(&doc, &styles);
    let mut fonts = FontSystem::new();
    let result = layout(
        Size::new(100.0, 100.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &Images::default(),
        },
    );
    let node = doc.get_element_by_id("test1").unwrap();
    let rect = result
        .boxes
        .iter()
        .find(|b| b.element.node == node && !b.anonymous)
        .unwrap()
        .border_box;
    assert!((rect.width - expected_left).abs() < 0.01);
    let scene = Rasterizer::default().prepare(&build_display_list(&result), &mut fonts);
    let mut pixels = vec![0xffffff; 100 * 100];
    scene.paint_software(
        &mut pixels,
        100,
        100,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        (0.0, 0.0),
        1.0,
        &Default::default(),
    );
    assert!(
        pixels.iter().all(|p| *p == 0x008000),
        "green-square reference mismatch"
    );
}
#[test]
fn wpt_flex_shrink_positive_number() {
    green(include_bytes!("wpt/flex-shrink-001.html"), 60.0);
}
#[test]
fn wpt_flex_shrink_negative_number_is_invalid() {
    green(include_bytes!("wpt/flex-shrink-002.html"), 50.0);
}
