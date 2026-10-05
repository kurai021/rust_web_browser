use cosmic_text::FontSystem;
use layout::*;
use std::sync::Arc;

fn page(
    markup: &str,
    css: &str,
    width: f32,
    images: &Images,
) -> (html::Document, RenderTree, LayoutResult) {
    let url = "https://example.test/docs/page".parse().unwrap();
    let doc = html::parse_full(markup.as_bytes(), &url, Default::default());
    let sheet = css::parse_stylesheet(
        &format!("html,body,p{{margin:0;padding:0}} {css}"),
        Some(&url),
    );
    let styles = css::Cascade::default().compute(&doc, &[sheet]);
    let mut tree = build_render_tree(&doc, &styles);
    let result = layout(
        Size::new(width, 300.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut FontSystem::new(),
            images,
        },
    );
    (doc, tree, result)
}
fn bbox<'a>(doc: &html::Document, result: &'a LayoutResult, id: &str) -> &'a LayoutBox {
    let node = doc.get_element_by_id(id).unwrap();
    result
        .boxes
        .iter()
        .find(|b| b.element.node == node && !b.anonymous)
        .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.1, "{a} != {b}");
}

#[test]
fn nested_content_box_and_auto_centering() {
    let (doc, _, result) = page(
        "<main id=m><div id=c></div></main>",
        "main{width:200px;margin:0 auto;padding:10px;border:2px solid red}#c{height:20px}",
        400.0,
        &Images::default(),
    );
    let main = bbox(&doc, &result, "m");
    let child = bbox(&doc, &result, "c");
    near(main.border_box.x, 88.0);
    near(main.border_box.width, 224.0);
    near(child.border_box.x, 100.0);
    near(child.border_box.y, 12.0);
    near(child.content_box.width, 200.0);
    near(main.border_box.height, 44.0);
}

#[test]
fn border_box_and_min_max_constraints() {
    let (doc, _, result) = page("<div id=a></div><div id=b></div>", "div{width:100px;height:40px;padding:10px;border:2px solid}#a{box-sizing:border-box}#b{max-width:60px;min-height:50px}", 300.0, &Images::default());
    near(bbox(&doc, &result, "a").content_box.width, 76.0);
    near(bbox(&doc, &result, "a").border_box.height, 40.0);
    near(bbox(&doc, &result, "b").border_box.width, 84.0);
    near(bbox(&doc, &result, "b").content_box.height, 50.0);
}

#[test]
fn parent_child_and_sibling_margins_collapse() {
    let (doc, _, result) = page(
        "<div id=p><div id=c></div></div><div id=s></div>",
        "#p{margin:20px 0}#c{height:10px;margin:30px 0 40px}#s{height:10px;margin-top:15px}",
        300.0,
        &Images::default(),
    );
    near(bbox(&doc, &result, "p").border_box.y, 30.0);
    near(bbox(&doc, &result, "c").border_box.y, 30.0);
    near(bbox(&doc, &result, "p").border_box.height, 10.0);
    near(bbox(&doc, &result, "s").border_box.y, 80.0);
}

#[test]
fn empty_and_negative_margins_form_one_collapsed_set() {
    let (doc, _, result) = page(
        "<div id=a></div><div id=e></div><div id=b></div>",
        "#a{height:10px;margin-bottom:20px}#e{margin:50px 0 -10px}#b{height:10px;margin-top:30px}",
        300.0,
        &Images::default(),
    );
    near(bbox(&doc, &result, "b").border_box.y, 50.0);
}

#[test]
fn overflow_and_borders_stop_parent_margin_collapsing() {
    let (doc, _, result) = page(
        "<div id=p><div id=c></div></div>",
        "#p{overflow:hidden;margin-top:10px}#c{height:10px;margin-top:30px}",
        300.0,
        &Images::default(),
    );
    near(bbox(&doc, &result, "p").border_box.y, 10.0);
    near(bbox(&doc, &result, "c").border_box.y, 40.0);
}

#[test]
fn percentage_height_requires_a_definite_containing_height() {
    let (doc, _, result) = page(
        "<div id=p><div id=a></div></div><section style='height:80px'><div id=b></div></section>",
        "#a,#b{height:50%}",
        300.0,
        &Images::default(),
    );
    near(bbox(&doc, &result, "a").content_box.height, 0.0);
    near(bbox(&doc, &result, "b").content_box.height, 40.0);
}

#[test]
fn anonymous_boxes_preserve_nested_inline_block_splitting() {
    let (_, tree, result) = page(
        "<div>before<span>inside<div>block</div>after</span>end</div>",
        "",
        300.0,
        &Images::default(),
    );
    assert!(tree
        .nodes
        .iter()
        .any(|n| matches!(n.kind, RenderKind::AnonymousBlock)));
    let text: Vec<_> = result
        .boxes
        .iter()
        .flat_map(|b| &b.content)
        .filter_map(|c| {
            if let Content::Text(t) = c {
                Some(t.text.as_str())
            } else {
                None
            }
        })
        .collect();
    for word in ["before", "inside", "block", "after", "end"] {
        assert!(text.contains(&word), "missing {word}: {text:?}");
    }
}

#[test]
fn generated_content_and_display_none() {
    let (_, tree, result) = page(
        "<p id=p>visible<span style='display:none'>secret</span></p>",
        "#p::before{content:'before ';color:red}#p::after{content:' after'}",
        300.0,
        &Images::default(),
    );
    assert!(tree
        .nodes
        .iter()
        .any(|n| n.element.pseudo == Some(css::selectors::PseudoElement::Before)));
    let text = result
        .boxes
        .iter()
        .flat_map(|b| &b.content)
        .filter_map(|c| {
            if let Content::Text(t) = c {
                Some(t.text.as_str())
            } else {
                None
            }
        })
        .collect::<String>();
    assert!(text.contains("before"));
    assert!(text.contains("after"));
    assert!(!text.contains("secret"));
}

#[test]
fn whitespace_modes_and_resize_change_line_boxes() {
    let markup = "<p>one  two\nthree four five six seven eight</p>";
    let (_, _, wide) = page(markup, "", 800.0, &Images::default());
    let (_, _, narrow) = page(markup, "", 100.0, &Images::default());
    assert!(narrow.stats.lines > wide.stats.lines);
    let (_, _, nowrap) = page(markup, "p{white-space:nowrap}", 100.0, &Images::default());
    assert_eq!(nowrap.stats.lines, 1);
    let (_, _, pre) = page(
        "<pre>one  two\nthree\n</pre>",
        "pre{margin:0}",
        100.0,
        &Images::default(),
    );
    assert_eq!(pre.stats.lines, 3);
}

#[test]
fn mixed_font_sizes_share_a_baseline_and_inline_padding_has_width() {
    let (doc, _, result) = page("<p id=p><span style='font-size:32px'>Big</span><span style='padding:0 10px;background:red'>small</span></p>", "", 400.0, &Images::default());
    let p = bbox(&doc, &result, "p");
    let texts: Vec<_> = p
        .content
        .iter()
        .filter_map(|c| {
            if let Content::Text(t) = c {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(texts.len(), 2);
    near(texts[0].glyphs[0].y as f32, texts[1].glyphs[0].y as f32);
    assert!(texts[1].rect.x >= texts[0].rect.right() + 9.9);
}

#[test]
fn intrinsic_images_css_dimensions_and_attribute_aspect_ratio() {
    let mut images = Images::default();
    images.insert(
        "https://example.test/p.png".parse().unwrap(),
        RasterImage::new(80, 40, vec![255; 80 * 40 * 4]).unwrap(),
    );
    let (doc, _, result) = page(
        "<img id=i src='/p.png' width=40><img id=j src='/p.png' style='width:20px'>",
        "",
        300.0,
        &images,
    );
    near(bbox(&doc, &result, "i").content_box.width, 40.0);
    near(bbox(&doc, &result, "i").content_box.height, 20.0);
    near(bbox(&doc, &result, "j").content_box.height, 10.0);
    assert!(result
        .boxes
        .iter()
        .flat_map(|b| &b.content)
        .any(|c| matches!(c, Content::Image { .. })));
}

#[test]
fn overflow_hit_testing_and_scroll_are_offset_only() {
    let (doc, _, result) = page(
        "<div id=scroll><p style='height:40px'>first</p><p><a href='/next'>next</a></p></div>",
        "#scroll{height:30px;overflow:auto}",
        300.0,
        &Images::default(),
    );
    let b = bbox(&doc, &result, "scroll");
    let hit = result
        .hits
        .iter()
        .find(|h| h.href.path() == "/next")
        .unwrap();
    let mut offsets = ScrollOffsets::default();
    assert!(result
        .hit_link(hit.rect.x + 1.0, hit.rect.y + 1.0, &offsets)
        .is_none());
    let before = result.stats.boxes;
    assert!(result.scroll_at(
        b.padding_box.x + 1.0,
        b.padding_box.y + 1.0,
        40.0,
        &mut offsets
    ));
    let dy = offsets[&b.element].1;
    assert!(result
        .hit_link(hit.rect.x + 1.0, hit.rect.y + 1.0 - dy, &offsets)
        .is_some());
    assert_eq!(result.stats.boxes, before);
    assert!(result.content_size.height < b.scroll_size.height + 1.0);
}

#[test]
fn rtl_and_shaped_link_geometry_are_retained() {
    let (_, _, result) = page(
        "<p style='direction:rtl'>مرحبا <a href='/next'>بالعالم</a> Hello</p>",
        "",
        300.0,
        &Images::default(),
    );
    assert!(result.stats.glyphs > 0);
    let offsets = ScrollOffsets::default();
    for hit in &result.hits {
        assert_eq!(
            result.hit_link(
                hit.rect.x + hit.rect.width / 2.0,
                hit.rect.y + hit.rect.height / 2.0,
                &offsets
            ),
            Some(hit.href.clone())
        );
    }
}

#[test]
fn viewport_geometry_is_finite_on_invalid_sizes() {
    let (_, _, result) = page(
        "<p>safe</p>",
        "p{width:calc(100% - 400px);padding:2px}",
        f32::NAN,
        &Images::default(),
    );
    assert!(result
        .boxes
        .iter()
        .all(|b| b.border_box.x.is_finite() && b.border_box.height.is_finite()));
}

#[test]
fn decoded_image_data_is_validated() {
    assert!(RasterImage::new(2, 2, vec![0; 3]).is_none());
    let image: Arc<RasterImage> = RasterImage::new(1, 1, vec![0; 4]).unwrap();
    assert_eq!(image.size(), Size::new(1.0, 1.0));
}

#[test]
fn depth_cap_keeps_readable_content_without_recursive_overflow() {
    let url = "https://example.test/".parse().unwrap();
    let input = format!("{}readable{}", "<div>".repeat(540), "</div>".repeat(540));
    let doc = html::parse_full(
        input.as_bytes(),
        &url,
        html::ParseOpts {
            max_depth: 1024,
            ..Default::default()
        },
    );
    let styles = css::Cascade::default().compute(&doc, &[]);
    let mut tree = build_render_tree(&doc, &styles);
    assert!(tree.depth_fallbacks > 0);
    let result = layout(
        Size::new(400.0, 300.0),
        &mut tree,
        LayoutOpts {
            font_system: &mut FontSystem::new(),
            images: &Images::default(),
        },
    );
    assert!(result.stats.glyphs > 0);
}

#[test]
fn display_contents_retains_text_and_video_aspect_ratio_and_image_margins_work() {
    let (doc, _, result) = page("<div style='display:contents'>kept text</div><video id=v style='width:160px;aspect-ratio:16 / 9'></video><img id=i width=40 height=20 style='display:block;margin:0 auto' alt=x>", "", 300.0, &Images::default());
    assert!(result
        .boxes
        .iter()
        .flat_map(|b| &b.content)
        .any(|c| matches!(c,Content::Text(t) if t.text=="kept")));
    near(bbox(&doc, &result, "v").content_box.height, 90.0);
    near(bbox(&doc, &result, "i").border_box.x, 130.0);
}
