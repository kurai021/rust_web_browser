use cosmic_text::FontSystem;
use layout::*;
fn render(d: &html::Document, css: &str) -> RenderTree {
    let sheet = css::parse_stylesheet(css, d.url.as_ref());
    let styles = css::Cascade::default().compute(d, &[sheet]);
    build_render_tree(d, &styles)
}
fn compare(a: &LayoutResult, b: &LayoutResult) {
    let rect = |a: Rect, b: Rect| {
        for (a, b) in [
            (a.x, b.x),
            (a.y, b.y),
            (a.width, b.width),
            (a.height, b.height),
        ] {
            assert!((a - b).abs() < 0.001, "{a} != {b}");
        }
    };
    assert_eq!(a.boxes.len(), b.boxes.len());
    for (a, b) in a.boxes.iter().zip(&b.boxes) {
        assert_eq!(a.element, b.element);
        rect(a.border_box, b.border_box);
        rect(a.content_box, b.content_box);
        let text = |b: &LayoutBox| {
            b.content
                .iter()
                .filter_map(|c| match c {
                    Content::Text(t) => Some((
                        t.text.clone(),
                        t.rect,
                        t.glyphs
                            .iter()
                            .map(|g| (g.x, g.y, g.color))
                            .collect::<Vec<_>>(),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let a = text(a);
        let b = text(b);
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(a.0, b.0);
            rect(a.1, b.1);
            assert_eq!(a.2, b.2);
        }
    }
    rect(
        Rect::new(0.0, 0.0, a.content_size.width, a.content_size.height),
        Rect::new(0.0, 0.0, b.content_size.width, b.content_size.height),
    );
    assert_eq!(a.paint_order, b.paint_order);
    assert_eq!(a.hits.len(), b.hits.len());
    for (a, b) in a.hits.iter().zip(&b.hits) {
        rect(a.rect, b.rect);
        assert_eq!(a.ancestors, b.ancestors);
    }
}
#[test]
fn text_mutation_reflows_its_branch_and_translates_unchanged_siblings() {
    let url = "https://example.test/".parse().unwrap();
    let mut d=html::parse_full(b"<main><section id=a>short</section><section id=b>unchanged sibling</section><section id=c>last</section></main>",&url,Default::default());
    let css = "html,body,main,section{margin:0}main{width:100px}section{padding:3px}";
    let mut cache = LayoutCache::default();
    let mut fonts = FontSystem::new();
    let images = Images::default();
    let mut tree = render(&d, css);
    let first = cache.layout(
        Size::new(400.0, 300.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    d.set_text_content(
        d.get_element_by_id("a").unwrap(),
        "a much longer paragraph that wraps across several lines",
    )
    .unwrap();
    let mut tree = render(&d, css);
    let updated = cache.layout(
        Size::new(400.0, 300.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    let full = layout(
        Size::new(400.0, 300.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    compare(&updated, &full);
    assert!(updated.stats.reused_subtrees >= 2);
    assert!(updated.content_size.height > first.content_size.height);
    let again = cache.layout(
        Size::new(400.0, 300.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut fonts,
            images: &images,
        },
    );
    assert_eq!(again.stats.dirty_nodes, 0);
    assert_eq!(again.stats.reused_boxes, again.boxes.len());
}
#[test]
fn grid_constraints_styles_structure_resize_and_assets_invalidate_dependencies() {
    let url = "https://example.test/".parse().unwrap();
    let mut d = html::parse_full(
        b"<main><div id=a>first</div><div id=b>second</div><img id=i src='/image.png'></main>",
        &url,
        Default::default(),
    );
    let css = "html,body,main{margin:0}main{display:grid;grid-template-columns:1fr 1fr;gap:10px}";
    let mut cache = LayoutCache::default();
    let mut fonts = FontSystem::new();
    let mut images = Images::default();
    for step in 0..4 {
        if step == 1 {
            d.set_attribute(
                d.get_element_by_id("a").unwrap(),
                "style",
                "height:100px;color:red",
            )
            .unwrap();
        }
        if step == 2 {
            images.insert(
                url.join("image.png").unwrap(),
                RasterImage::new(10, 20, vec![255; 800]).unwrap(),
            );
        }
        let viewport = Size::new(if step == 3 { 300.0 } else { 400.0 }, 300.0);
        let mut tree = render(&d, css);
        let cached = cache.layout(
            viewport,
            &mut tree,
            LayoutOpts {
                font_system: &mut fonts,
                images: &images,
            },
        );
        let full = layout(
            viewport,
            &mut tree,
            LayoutOpts {
                font_system: &mut fonts,
                images: &images,
            },
        );
        compare(&cached, &full);
        if step == 3 {
            assert_eq!(cached.stats.reused_subtrees, 0);
        }
    }
}
#[test]
fn insertion_removal_and_z_order_do_not_reuse_stale_node_indices() {
    let url = "https://example.test/".parse().unwrap();
    let mut d = html::parse_full(
        b"<main id=m><p id=a>one</p><p id=b>two</p></main>",
        &url,
        Default::default(),
    );
    let css = "html,body,main,p{margin:0}main{position:relative}p{height:20px}";
    let mut fonts = FontSystem::new();
    let mut cache = LayoutCache::default();
    let images = Images::default();
    for step in 0..3 {
        if step == 1 {
            let node = d.create_element("p", html::Namespace::Html);
            d.set_text_content(node, "inserted").unwrap();
            d.append_checked(
                d.get_element_by_id("m").unwrap(),
                node,
                Some(d.get_element_by_id("a").unwrap()),
            )
            .unwrap();
        }
        if step == 2 {
            d.detach(d.get_element_by_id("a").unwrap());
            d.set_attribute(
                d.get_element_by_id("b").unwrap(),
                "style",
                "position:absolute;inset:0;z-index:5",
            )
            .unwrap();
        }
        let mut t = render(&d, css);
        let cached = cache.layout(
            Size::new(300.0, 200.0),
            &mut t,
            LayoutOpts {
                font_system: &mut fonts,
                images: &images,
            },
        );
        let full = layout(
            Size::new(300.0, 200.0),
            &mut t,
            LayoutOpts {
                font_system: &mut fonts,
                images: &images,
            },
        );
        compare(&cached, &full);
    }
}
