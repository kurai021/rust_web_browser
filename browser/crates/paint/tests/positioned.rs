use cosmic_text::FontSystem;
use layout::*;
use paint::*;
#[test]
fn paint_and_hit_order_agree_and_fixed_composition_does_not_layout() {
    let url = "https://example.test/".parse().unwrap();
    let d=html::parse_full(b"<div id=p><div><div id=high></div></div><div id=low></div></div><div id=f></div><div style='height:500px'></div>",&url,Default::default());
    let sheet=css::parse_stylesheet("html,body{margin:0}#p{position:relative;width:100px;height:100px}#high,#low{position:absolute;inset:0}#high{background:red;z-index:5}#low{background:blue;z-index:3}#f{position:fixed;top:10px;left:120px;width:30px;height:20px;background:green}",Some(&url));
    let styles = css::Cascade::default().compute(&d, &[sheet]);
    let mut fonts = FontSystem::new();
    let mut tree = build_render_tree(&d, &styles);
    let result = layout(
        Size::new(200.0, 100.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &Images::default(),
        },
    );
    let scene = Rasterizer::default().prepare(&build_display_list(&result), &mut fonts);
    let mut pixels = vec![0xffffff; 200 * 100];
    scene.paint_software(
        &mut pixels,
        200,
        100,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        (0.0, 0.0),
        1.0,
        &ScrollOffsets::default(),
    );
    assert_eq!(pixels[20 * 200 + 20], 0xff0000);
    assert_eq!(pixels[15 * 200 + 125], 0x008000);
    scene.paint_software(
        &mut pixels,
        200,
        100,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        (0.0, -150.0),
        1.0,
        &ScrollOffsets::default(),
    );
    assert_eq!(pixels[15 * 200 + 125], 0x008000);
    assert_eq!(
        result
            .hit_element_scrolled(125.0, 165.0, (0.0, 150.0), &ScrollOffsets::default())
            .unwrap()
            .node,
        d.get_element_by_id("f").unwrap()
    );
}
