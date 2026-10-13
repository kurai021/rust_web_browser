use cosmic_text::FontSystem;
use layout::*;
fn page(html: &str, rules: &str, width: f32, height: f32) -> (html::Document, LayoutResult) {
    let url = "https://example.test/".parse().unwrap();
    let doc = html::parse_full(html.as_bytes(), &url, Default::default());
    let css = css::parse_stylesheet(
        &format!("html,body,p{{margin:0;padding:0}}{rules}"),
        Some(&url),
    );
    let styles = css::Cascade::default().compute(&doc, &[css]);
    let mut tree = build_render_tree(&doc, &styles);
    let result = layout(
        Size::new(width, height),
        &mut tree,
        LayoutOpts {
            font_system: &mut FontSystem::new(),
            images: &Images::default(),
        },
    );
    (doc, result)
}
fn bbox<'a>(d: &html::Document, l: &'a LayoutResult, id: &str) -> &'a LayoutBox {
    let id = d.get_element_by_id(id).unwrap();
    l.boxes
        .iter()
        .find(|b| b.element.node == id && !b.anonymous)
        .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.15, "{a} != {b}");
}
#[test]
fn flex_grow_shrink_and_gap_use_the_inner_main_size() {
    let (d,l)=page("<div id=f><div id=a></div><div id=b></div></div>","#f{display:flex;width:300px;gap:10px}#a{flex:1 1 50px;height:20px}#b{flex:2 1 50px;height:40px}",400.0,300.0);
    near(bbox(&d, &l, "a").content_box.width, 50.0 + 190.0 / 3.0);
    near(bbox(&d, &l, "b").content_box.width, 50.0 + 380.0 / 3.0);
    near(
        bbox(&d, &l, "b").border_box.x,
        bbox(&d, &l, "a").border_box.right() + 10.0,
    );
    near(bbox(&d, &l, "f").content_box.height, 40.0);
    let (d, l) = page(
        "<div id=f><div id=a></div><div id=b></div></div>",
        "#f{display:flex;width:150px}#a,#b{width:100px;min-width:0;height:10px}",
        400.0,
        300.0,
    );
    near(bbox(&d, &l, "a").content_box.width, 75.0);
    near(bbox(&d, &l, "b").border_box.x, 75.0);
}
#[test]
fn flex_auto_margins_and_no_wrap_cross_alignment() {
    let (d,l)=page("<div id=f><div id=a></div><div id=b></div></div>","#f{display:flex;width:200px;height:100px;align-content:center;align-items:start}#a{width:30px;height:20px;margin-top:auto;margin-bottom:auto}#b{width:40px;height:20px;margin-left:auto}",300.0,200.0);
    near(bbox(&d, &l, "a").border_box.y, 40.0);
    near(bbox(&d, &l, "b").border_box.x, 160.0);
    near(bbox(&d, &l, "b").border_box.y, 0.0);
}
#[test]
fn flex_wrap_stretch_reverse_and_column() {
    let (d, l) = page(
        "<div id=f><span id=a>A</span><span id=b>B</span><span id=c>C</span></div>",
        "#f{display:flex;flex-wrap:wrap;width:200px;gap:10px}#a,#b,#c{flex:0 0 90px;height:20px}",
        400.0,
        300.0,
    );
    near(bbox(&d, &l, "b").border_box.x, 100.0);
    near(bbox(&d, &l, "c").border_box.y, 30.0);
    near(bbox(&d, &l, "f").content_box.height, 50.0);
    let (d,l)=page("<div id=f><div id=a></div><div id=b></div></div>","#f{display:flex;flex-direction:row-reverse;width:200px;justify-content:space-between}#a,#b{width:40px;height:20px}",400.0,300.0);
    near(bbox(&d, &l, "a").border_box.x, 160.0);
    near(bbox(&d, &l, "b").border_box.x, 0.0);
    let (d,l)=page("<div id=f><div id=a></div><div id=b></div></div>","#f{display:flex;flex-direction:column;width:100px;height:200px;gap:10px}#a,#b{flex:1 1 0px}",400.0,300.0);
    near(bbox(&d, &l, "a").content_box.height, 95.0);
    near(bbox(&d, &l, "b").border_box.y, 105.0);
    near(bbox(&d, &l, "a").content_box.width, 100.0);
}
#[test]
fn nested_flex_navbar_intrinsic_basis_includes_its_gaps() {
    let (d,l)=page("<header><strong>title</strong><nav id=n><a></a><a></a><a></a><a></a></nav></header>","header{display:flex;width:600px;justify-content:space-between}nav{display:flex;gap:10px;flex-wrap:wrap}a{width:40px;height:20px}",600.0,300.0);
    near(bbox(&d, &l, "n").content_box.width, 190.0);
    near(bbox(&d, &l, "n").content_box.height, 20.0);
}
#[test]
fn grid_fraction_percentage_areas_and_spans() {
    let (d,l)=page("<div id=g><div id=a></div><div id=b></div><div id=c></div></div>","#g{display:grid;width:400px;grid-template-columns:25% 1fr 2fr;gap:10px}#a,#b,#c{height:20px}",500.0,300.0);
    near(bbox(&d, &l, "a").content_box.width, 100.0);
    near(bbox(&d, &l, "b").content_box.width, 280.0 / 3.0);
    near(bbox(&d, &l, "c").content_box.width, 560.0 / 3.0);
    let (d,l)=page("<div id=g><div id=head></div><div id=side></div><div id=main></div></div>","#g{display:grid;grid-template-columns:100px 200px;grid-template-rows:30px 80px;grid-template-areas:'head head' 'side main';gap:10px}#head{grid-area:head}#side{grid-area:side}#main{grid-area:main}",310.0,300.0);
    near(bbox(&d, &l, "head").border_box.width, 310.0);
    near(bbox(&d, &l, "main").border_box.x, 110.0);
    near(bbox(&d, &l, "main").border_box.y, 40.0);
    near(bbox(&d, &l, "g").content_box.height, 120.0);
}
#[test]
fn grid_auto_fit_and_implicit_rows_wrap_cards() {
    let (d,l)=page("<div id=g><div id=a></div><div id=b></div><div id=c></div><div id=d></div></div>","#g{display:grid;width:320px;grid-template-columns:repeat(auto-fit,minmax(100px,1fr));gap:10px}#g>div{height:20px}",400.0,300.0);
    near(bbox(&d, &l, "a").content_box.width, 100.0);
    near(bbox(&d, &l, "c").border_box.x, 220.0);
    near(bbox(&d, &l, "d").border_box.y, 30.0);
}
#[test]
fn adversarial_tracks_and_extents_remain_bounded_and_finite() {
    let (_,l)=page("<div><div>finite text</div></div>","div{position:relative;left:1e38px;top:1e38px;padding:1e38px;border:1e38px solid;width:1e38px}",400.0,300.0);
    assert!(l.boxes.iter().all(|b| [
        b.border_box.x,
        b.border_box.y,
        b.border_box.width,
        b.border_box.height
    ]
    .iter()
    .all(|v| v.is_finite())));
    let (_, l) = page(
        "<main><p>kept</p><p>safe</p></main>",
        "main{display:grid;grid-template-columns:repeat(1000000,1fr)}p{grid-column:4096/span 128}",
        400.0,
        300.0,
    );
    assert!(l.boxes.len() < 1000);
    assert!(l.stats.truncated);
}
#[test]
fn relative_and_absolute_boxes_do_not_advance_normal_flow() {
    let (d,l)=page("<div id=p><div id=r></div><div id=a></div><div id=n></div></div>","#p{position:relative;width:200px;height:100px;padding:10px;border:2px solid}#r{position:relative;left:5px;top:7px;height:20px}#a{position:absolute;right:0;bottom:0;width:30px;height:40px}#n{height:10px}",400.0,300.0);
    near(bbox(&d, &l, "r").border_box.x, 17.0);
    near(bbox(&d, &l, "r").border_box.y, 19.0);
    near(bbox(&d, &l, "n").border_box.y, 32.0);
    near(bbox(&d, &l, "a").border_box.x, 192.0);
    near(bbox(&d, &l, "a").border_box.y, 82.0);
}
#[test]
fn float_exclusions_wrap_text_and_clear_uses_the_margin_box() {
    let (d,l)=page("<div id=root><div id=float></div><p id=text>one two three four five six seven eight nine ten eleven twelve</p><div id=clear></div></div>","#root{width:240px}#float{float:left;width:80px;height:50px;margin-right:10px}#clear{clear:both;height:10px}",400.0,300.0);
    let text = bbox(&d, &l, "text");
    let run = text
        .content
        .iter()
        .find_map(|c| match c {
            Content::Text(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert!(run.rect.x >= 90.0);
    assert!(bbox(&d, &l, "clear").border_box.y >= 50.0);
}
#[test]
fn table_rows_columns_and_spanning_cells_align() {
    let (d,l)=page("<table id=t><tbody><tr><td id=a rowspan=2>A</td><td id=b>B</td></tr><tr><td id=c>C</td></tr><tr><td id=d colspan=2>D</td></tr></tbody></table>","#t{width:240px;border-spacing:0;table-layout:fixed}td{height:20px;padding:0}",400.0,300.0);
    near(bbox(&d, &l, "a").content_box.width, 120.0);
    near(bbox(&d, &l, "b").border_box.x, 120.0);
    near(bbox(&d, &l, "c").border_box.x, 120.0);
    assert!(bbox(&d, &l, "c").border_box.y > bbox(&d, &l, "b").border_box.y);
    near(bbox(&d, &l, "d").content_box.width, 240.0);
    near(
        bbox(&d, &l, "a").border_box.height,
        bbox(&d, &l, "b").border_box.height + bbox(&d, &l, "c").border_box.height,
    );
}
#[test]
fn fixed_layers_escape_ancestor_scroll_clips_and_hit_in_viewport_coordinates() {
    let (d,l)=page("<div id=s><a id=f href='/fixed'>fixed</a></div><div style='height:500px'></div>","#s{height:20px;overflow:hidden}#f{position:fixed;left:10px;top:5px;width:40px;height:20px;background:red}",200.0,100.0);
    let fixed = bbox(&d, &l, "f");
    assert!(fixed.fixed);
    near(fixed.border_box.y, 5.0);
    let hit = l
        .hit_element_scrolled(15.0, 110.0, (0.0, 100.0), &ScrollOffsets::default())
        .unwrap();
    let element = if matches!(d.get(hit.node).data, html::NodeData::Text(_)) {
        d.get(hit.node).parent.unwrap()
    } else {
        hit.node
    };
    assert_eq!(element, d.get_element_by_id("f").unwrap());
}
#[test]
fn stacking_contexts_hoist_positioned_descendants_but_isolate_opacity() {
    let (d,l)=page("<div id=root><div><div id=high></div></div><div id=middle></div><div id=opacity><div id=trapped></div></div></div>","#root{position:relative;height:100px}#high,#middle,#opacity,#trapped{position:absolute;inset:0}#high{z-index:4;background:red}#middle{z-index:3;background:green}#opacity{opacity:.5;z-index:1}#trapped{z-index:99;background:blue}",200.0,100.0);
    let key = |id: &str| {
        l.boxes
            .iter()
            .position(|b| b.element.node == d.get_element_by_id(id).unwrap() && !b.anonymous)
            .unwrap()
    };
    assert!(l.paint_rank[key("high")] > l.paint_rank[key("middle")]);
    assert!(l.paint_rank[key("trapped")] < l.paint_rank[key("middle")]);
    assert_eq!(
        l.hit_element(20.0, 20.0, &ScrollOffsets::default())
            .unwrap()
            .node,
        d.get_element_by_id("high").unwrap()
    );
}
