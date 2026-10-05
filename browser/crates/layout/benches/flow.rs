use cosmic_text::FontSystem;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use layout::{build_render_tree, layout, Images, LayoutOpts, Size};

fn flow(c: &mut Criterion) {
    let url = "http://localhost/bench".parse().unwrap();
    let input = format!("<style>main{{max-width:720px;margin:auto;padding:16px}}p{{margin:12px 0}}span{{font-weight:bold}}</style><main>{}</main>", "<p>A paragraph with <span>shaped text</span> and a link to test flow wrapping and nested boxes.</p>".repeat(100));
    let doc = html::parse_full(input.as_bytes(), &url, Default::default());
    let sheet = css::parse_stylesheet(
        &doc.text_content(doc.get_elements_by_tag_name("style")[0]),
        Some(&url),
    );
    let styles = css::Cascade::default().compute(&doc, &[sheet]);
    let mut tree = build_render_tree(&doc, &styles);
    let mut fonts = FontSystem::new();
    let images = Images::default();
    c.bench_function("flow/100_paragraphs", |b| {
        b.iter(|| {
            black_box(layout(
                Size::new(1000.0, 700.0),
                &mut tree,
                LayoutOpts {
                    font_system: &mut fonts,
                    images: &images,
                },
            ))
        })
    });
    c.bench_function("flow/resize_100_paragraphs", |b| {
        b.iter(|| {
            black_box(layout(
                Size::new(420.0, 700.0),
                &mut tree,
                LayoutOpts {
                    font_system: &mut fonts,
                    images: &images,
                },
            ))
        })
    });
}
criterion_group!(benches, flow);
criterion_main!(benches);
