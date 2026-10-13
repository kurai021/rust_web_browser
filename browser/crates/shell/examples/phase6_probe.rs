//! Deterministic layout/invalidation evidence over the offline workshop.
use cosmic_text::FontSystem;
use layout::*;
use std::time::Instant;
fn tree(d: &html::Document, s: &css::Stylesheet) -> RenderTree {
    let styles = css::Cascade::default().compute(d, std::slice::from_ref(s));
    build_render_tree(d, &styles)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = "http://localhost/".parse()?;
    let mut doc = html::parse_full(
        include_bytes!("../tests/fixtures/advanced-demo/index.html"),
        &url,
        Default::default(),
    );
    let sheet = css::parse_stylesheet(
        include_str!("../tests/fixtures/advanced-demo/site.css"),
        Some(&url),
    );
    let mut fonts = FontSystem::new();
    let mut cache = LayoutCache::default();
    let images = Images::default();
    let mut render = tree(&doc, &sheet);
    let start = Instant::now();
    let first = cache.layout(
        Size::new(1000.0, 800.0),
        &mut render,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    let first_ms = start.elapsed().as_secs_f64() * 1000.0;
    let id = doc
        .get_element_by_id("mutation")
        .ok_or("missing mutation target")?;
    doc.set_text_content(id,"A longer paragraph changes the first card and its row. Unchanged cards and sections retain their geometry when the constraints remain compatible.")?;
    let mut render = tree(&doc, &sheet);
    let start = Instant::now();
    let changed = cache.layout(
        Size::new(1000.0, 800.0),
        &mut render,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    let dirty_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let full = layout(
        Size::new(1000.0, 800.0),
        &mut render,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    let full_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let resized = cache.layout(
        Size::new(480.0, 800.0),
        &mut render,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    let resize_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{{\"first_ms\":{first_ms:.3},\"dirty_ms\":{dirty_ms:.3},\"full_ms\":{full_ms:.3},\"resize_ms\":{resize_ms:.3},\"first_boxes\":{},\"changed_boxes\":{},\"dirty_nodes\":{},\"reused_boxes\":{},\"reused_subtrees\":{},\"full_boxes\":{},\"resized_boxes\":{}}}",first.boxes.len(),changed.boxes.len(),changed.stats.dirty_nodes,changed.stats.reused_boxes,changed.stats.reused_subtrees,full.boxes.len(),resized.boxes.len());
    Ok(())
}
