//! Explicit offline evidence harness: snapshots, 1 MiB stages and K4 baseline.
use base64::Engine;
use cosmic_text::FontSystem;
use layout::{RasterImage, Rect, Size};
use paint::{gpu::Gpu, Quad, Scene};
use shell::page::Page;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).map(std::path::PathBuf::from);
    let dot = RasterImage::new(1, 1, vec![18, 106, 188, 255]).ok_or("dot dimensions")?;
    let encoded = paint::images::encode_png(&dot)?;
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encoded)
    );
    println!("fixture_png={data_url}");
    let head = net::ResponseHead {
        url: "http://localhost/flow/index.html".parse()?,
        status: 200,
        content_type: Some("text/html".into()),
    };
    let document = html::parse_full(
        include_bytes!("../tests/fixtures/flow-demo/index.html"),
        &head.url,
        Default::default(),
    );
    let mut page = Page::preview(&head, document);
    page.stylesheets.push(css::parse_stylesheet(
        include_str!("../tests/fixtures/flow-demo/site.css"),
        Some(&head.url),
    ));
    let landscape =
        paint::images::decode_image(include_bytes!("../tests/fixtures/flow-demo/landscape.svg"))?;
    page.image_cache
        .insert(head.url.join("landscape.svg")?, Some(landscape));
    page.image_cache.insert(data_url.parse()?, Some(dot));
    let mut fonts = FontSystem::new();
    for width in [1000.0, 420.0] {
        let styles = page.computed_styles(css::Environment {
            width,
            height: 700.0,
            dark: false,
        });
        let frame =
            shell::viewport::headless_frame(&page, &styles, Size::new(width, 700.0), &mut fonts);
        println!("demo width={width} boxes={} lines={} glyphs={} layout_ms={:.3} paint_ms={:.3} hash={:016x}", frame.layout.stats.boxes, frame.layout.stats.lines, frame.layout.stats.glyphs, frame.layout_ms, frame.paint_ms, frame.hash);
        if let Some(output) = &output {
            paint::images::save_snapshot(
                &output.join(format!("software-{}.png", width as u32)),
                width as u32,
                700,
                &frame.pixels,
            )?;
        }
    }
    let paragraph = "<p>A synthetic paragraph with <b>shaped words</b> and a <a href='/next'>link</a> for measuring the complete local document pipeline.</p>";
    let input = format!(
        "<style>body{{font-family:sans-serif}}p{{margin:8px 0}}</style>{}",
        paragraph.repeat((1024usize * 1024).div_ceil(paragraph.len()))
    );
    let start = Instant::now();
    let document = html::parse_full(input.as_bytes(), &head.url, Default::default());
    let parse_ms = start.elapsed().as_secs_f64() * 1000.0;
    let page = Page::preview(&head, document);
    let start = Instant::now();
    let styles = page.computed_styles(Default::default());
    let style_ms = start.elapsed().as_secs_f64() * 1000.0;
    let frame =
        shell::viewport::headless_frame(&page, &styles, Size::new(1000.0, 700.0), &mut fonts);
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .map(str::to_owned)
        })
        .unwrap_or_default();
    println!("synthetic bytes={} nodes={} parse_ms={parse_ms:.3} style_ms={style_ms:.3} layout_ms={:.3} paint_ms={:.3} boxes={} lines={} glyphs={} {rss}", input.len(), page.document.node_count(), frame.layout_ms, frame.paint_ms, frame.layout.stats.boxes, frame.layout.stats.lines, frame.layout.stats.glyphs);
    let image = RasterImage::new(480, 400, [28, 120, 200, 170].repeat(480 * 400))
        .ok_or("benchmark image")?;
    let mut quads = vec![Quad::image(Rect::new(50.0, 90.0, 960.0, 600.0), image)];
    quads.extend((0..80).map(|i| {
        Quad::solid(
            Rect::new(20.0, i as f32 * 20.0, 700.0, 16.0),
            css::values::Color {
                r: 40,
                g: 100,
                b: 180,
                a: 160,
            },
        )
    }));
    let scene = Scene::from_quads(quads);
    let clip = Rect::new(0.0, 0.0, 1024.0, 768.0);
    let mut pixels = vec![0xffffff; 1024 * 768];
    let mut cpu = Vec::new();
    let mut gpu_times = Vec::new();
    let mut gpu = pollster::block_on(Gpu::offscreen())?;
    let _ = gpu.snapshot(
        1024,
        768,
        &scene.visible_quads(clip, (0.0, -100.0), 1.0, &Default::default()),
    )?;
    for i in 0..60 {
        let origin = (0.0, -(i % 20) as f32 * 10.0);
        pixels.fill(0xffffff);
        let start = Instant::now();
        scene.paint_software(
            &mut pixels,
            1024,
            768,
            clip,
            origin,
            1.0,
            &Default::default(),
        );
        cpu.push(start.elapsed().as_secs_f64() * 1000.0);
        let quads = scene.visible_quads(clip, origin, 1.0, &Default::default());
        let start = Instant::now();
        let _ = gpu.snapshot(1024, 768, &quads)?;
        gpu_times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    cpu.sort_by(f64::total_cmp);
    gpu_times.sort_by(f64::total_cmp);
    println!("K4 baseline adapter={} frames=60 cpu_median_ms={:.3} cpu_p95_ms={:.3} gpu_readback_median_ms={:.3} gpu_p95_ms={:.3} gain_percent={:.1}", gpu.adapter_name, cpu[30], cpu[57], gpu_times[30], gpu_times[57], (1.0 - gpu_times[30] / cpu[30]) * 100.0);
    Ok(())
}
