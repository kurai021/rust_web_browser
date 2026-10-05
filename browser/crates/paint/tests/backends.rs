use cosmic_text::FontSystem;
use layout::{Images, LayoutOpts, RasterImage, Rect, ScrollOffsets, Size};
use paint::*;

fn fixture() -> (Scene, layout::LayoutResult) {
    let url = "http://localhost/demo".parse().unwrap();
    let doc = html_doc(&url);
    let sheet = css::parse_stylesheet("html,body{margin:0}main{width:110px;padding:5px;border:3px solid #135;background:#e9effa}p{margin:0 0 8px;font:16px sans-serif}a{color:#a30}#clip{height:30px;overflow:auto;background:#cde}span{background:#fea}", Some(&url));
    let styles = css::Cascade::default().compute(&doc, &[sheet]);
    let mut tree = layout::build_render_tree(&doc, &styles);
    let mut fonts = FontSystem::new();
    let result = layout::layout(
        Size::new(160.0, 200.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &Images::default(),
        },
    );
    let list = build_display_list(&result);
    let scene = Rasterizer::default().prepare(&list, &mut fonts);
    (scene, result)
}
fn html_doc(url: &url::Url) -> html::Document {
    html::parse_full(b"<main><p>Text <span>colors</span> <a href='/next'>Link</a></p><div id='clip'><p>first line</p><p>second line</p></div></main>", url, Default::default())
}

#[test]
fn clips_alpha_bilinear_scaling_and_rounded_rects_have_scalar_snapshots() {
    let image = RasterImage::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut round = Quad::solid(Rect::new(0.0, 0.0, 4.0, 4.0), css::values::Color::BLACK);
    round.rounded = Some((round.rect, 2.0));
    let scene = Scene::from_quads(vec![
        round,
        Quad::image(Rect::new(4.0, 0.0, 4.0, 2.0), image),
    ]);
    let mut pixels = vec![0xffffff; 8 * 4];
    scene.paint_software(
        &mut pixels,
        8,
        4,
        Rect::new(0.0, 0.0, 8.0, 4.0),
        (0.0, 0.0),
        1.0,
        &ScrollOffsets::default(),
    );
    assert_eq!(pixels[0], 0xffffff);
    assert_eq!(pixels[1], 0);
    assert_eq!(pixels[4], 0xff0000);
    assert_eq!(pixels[7], 0x0000ff);
    assert_eq!(pixels[5], 0xbf0040);
    assert_eq!(pixels[6], 0x4000bf);
}

#[test]
fn display_list_glyphs_are_prepared_once_and_scroll_does_not_layout() {
    let (scene, result) = fixture();
    let before = (result.stats.boxes, result.stats.lines, result.stats.glyphs);
    let mut pixels = vec![0xffffff; 160 * 200];
    let clip = Rect::new(0.0, 0.0, 160.0, 200.0);
    scene.paint_software(
        &mut pixels,
        160,
        200,
        clip,
        (0.0, 0.0),
        1.0,
        &Default::default(),
    );
    let initial = pixels.clone();
    scene.paint_software(
        &mut pixels,
        160,
        200,
        clip,
        (0.0, -20.0),
        1.0,
        &Default::default(),
    );
    assert_ne!(pixels, initial);
    assert_eq!(
        before,
        (result.stats.boxes, result.stats.lines, result.stats.glyphs)
    );
    assert!(scene.quad_count() > result.stats.glyphs / 2);
}

#[test]
#[ignore = "requires a Vulkan/GL adapter; run explicitly in Phase 4 verification"]
fn gpu_and_software_render_the_same_display_list() {
    let (scene, result) = fixture();
    let mut gpu = pollster::block_on(gpu::Gpu::offscreen()).unwrap();
    println!("GPU parity adapter: {}", gpu.adapter_name);
    let clip = Rect::new(0.0, 0.0, 160.0, 200.0);
    let scroll = result
        .boxes
        .iter()
        .find(|b| b.overflow == layout::Overflow::Auto)
        .unwrap();
    let mut offsets = ScrollOffsets::default();
    offsets.insert(scroll.element, (0.0, 12.0));
    for scale in [1.0, 1.5] {
        let quads = scene.visible_quads(clip, (0.0, -4.0), scale, &offsets);
        let rgba = gpu.snapshot(160, 200, &quads).unwrap();
        let mut pixels = vec![0xffffff; 160 * 200];
        scene.paint_software(&mut pixels, 160, 200, clip, (0.0, -4.0), scale, &offsets);
        let max_delta = pixels
            .iter()
            .zip(rgba.chunks_exact(4))
            .flat_map(|(&cpu, p)| {
                [(cpu >> 16) as u8, (cpu >> 8) as u8, cpu as u8]
                    .into_iter()
                    .zip(p[..3].iter().copied())
                    .map(|(a, b)| a.abs_diff(b))
            })
            .max()
            .unwrap();
        assert!(
            max_delta <= 3,
            "backend pixel delta {max_delta} at scale {scale}"
        );
    }
}

#[test]
fn decoder_caps_failures_svg_external_resources_and_cache() {
    assert!(images::decode_image(include_bytes!("../fuzz/seeds/arc-budget.svg")).is_err());
    assert!(images::decode_image(b"not an image").is_err());
    assert!(images::decode_image(
        b"<svg xmlns='http://www.w3.org/2000/svg' width='9999999' height='9'/>"
    )
    .is_err());
    let image = images::decode_image(b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='red'/><image href='file:///etc/passwd'/></svg>").unwrap();
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    let cache = images::ImageCache::default();
    let url: url::Url = "http://localhost/i.svg".parse().unwrap();
    cache.insert(url.clone(), Some(image));
    assert_eq!(cache.bytes(), 64);
    assert!(cache.get(&url).unwrap().is_some());
    cache.insert(url.clone(), None);
    assert_eq!(cache.bytes(), 0);
    assert!(cache.get(&url).unwrap().is_none());
}

#[test]
fn png_jpeg_gif_and_webp_decode_through_the_bounded_path() {
    for format in [
        image::ImageFormat::Png,
        image::ImageFormat::Jpeg,
        image::ImageFormat::Gif,
        image::ImageFormat::WebP,
    ] {
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            4,
            image::Rgb([30, 110, 200]),
        ));
        let mut encoded = std::io::Cursor::new(Vec::new());
        source.write_to(&mut encoded, format).unwrap();
        let decoded = images::decode_image(encoded.get_ref()).unwrap();
        assert_eq!((decoded.width, decoded.height), (8, 4), "{format:?}");
        assert_eq!(decoded.rgba[3], 255);
    }
}
