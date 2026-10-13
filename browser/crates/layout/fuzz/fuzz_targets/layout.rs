#![no_main]
use libfuzzer_sys::fuzz_target;
use std::cell::RefCell;
thread_local! {static FONTS:RefCell<cosmic_text::FontSystem>=RefCell::new(cosmic_text::FontSystem::new());}
fuzz_target!(|bytes:&[u8]|{
    let Ok(source)=std::str::from_utf8(bytes)else{return;};
    let (markup,css)=source.split_once("\n---CSS---\n").unwrap_or((source,""));
    let url:url::Url="https://example.test/".parse().expect("constant URL");
    let doc=html::parse_full(markup.as_bytes(),&url,html::ParseOpts{max_nodes:256,max_depth:24,..Default::default()});
    let sheets=[css::parse_stylesheet(css,Some(&url))];let styles=css::Cascade::default().compute(&doc,&sheets);
    let mut tree=layout::build_render_tree(&doc,&styles);
    FONTS.with(|fonts|{let mut fonts=fonts.borrow_mut();let result=layout::layout(layout::Size::new(320.0,240.0),&mut tree,layout::LayoutOpts{font_system:&mut fonts,images:&Default::default()});
        assert!(result.boxes.iter().all(|b|[b.border_box.x,b.border_box.y,b.border_box.width,b.border_box.height].iter().all(|n|n.is_finite())));
        let _=result.hit_element(10.0,10.0,&Default::default());
    });
});
