use dom_bindings::{html, js::JsValue, policy::Policy};
use std::sync::Arc;
use web_api::{
    runtime::{CanvasImage, GeometryBox, GeometryRect},
    PageRuntime, RuntimeConfig,
};
fn runtime() -> PageRuntime {
    let url = "https://example.test/".parse().unwrap();
    let mut r=PageRuntime::new(html::parse_full(b"<body><div id=root><p id=child>old</p></div><canvas id=chart width=100 height=60></canvas><img id=source src=/image.png></body>",&url,Default::default()),url,Vec::new(),Default::default(),Policy::default(),RuntimeConfig::default()).unwrap();
    r.set_ready("complete");
    r
}
#[test]
fn mutation_records_batch_old_values_children_and_take_records() {
    let mut r = runtime();
    r.eval("var batches=[];var root=document.getElementById('root');var child=document.getElementById('child');var observer=new MutationObserver(records=>batches.push(records));observer.observe(root,{childList:true,subtree:true,attributes:true,attributeOldValue:true,characterData:true,characterDataOldValue:true});child.setAttribute('title','one');child.setAttribute('title','two');child.firstChild.textContent='new';root.appendChild(document.createElement('span'));").unwrap();
    assert_eq!(r.eval("batches.length+':'+batches[0].length+':'+batches[0][1].oldValue+':'+batches[0][2].oldValue+':'+batches[0][3].addedNodes[0].tagName;").unwrap(),JsValue::string("1:4:one:old:SPAN"));
    assert_eq!(r.eval("child.setAttribute('title','three');var taken=observer.takeRecords();taken.length+':'+taken[0].oldValue;").unwrap(),JsValue::string("1:two"));
    assert_eq!(r.eval("batches.length;").unwrap(), JsValue::Number(1.0));
    r.eval("observer.disconnect();child.setAttribute('title','four');")
        .unwrap();
    assert_eq!(r.eval("batches.length;").unwrap(), JsValue::Number(1.0));
}
#[test]
fn resize_and_intersection_follow_layout_and_scroll_without_duplicate_callbacks() {
    let mut r = runtime();
    r.eval("var sizes=[];var intersections=[];var target=document.getElementById('child');var resize=new ResizeObserver(e=>sizes.push(e[0].contentRect.width));resize.observe(target);var intersection=new IntersectionObserver(e=>intersections.push(e[0].isIntersecting),{threshold:[0,0.5,1]});intersection.observe(target);").unwrap();
    let node = r.document().get_element_by_id("child").unwrap();
    let rect = GeometryRect {
        x: 0.0,
        y: 20.0,
        width: 50.0,
        height: 30.0,
    };
    let viewport = GeometryRect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    r.geometry(
        vec![(
            node,
            GeometryBox {
                border: rect,
                content: rect,
                ..Default::default()
            },
        )],
        viewport,
    )
    .unwrap();
    r.viewport(viewport).unwrap();
    assert_eq!(
        r.eval("sizes.join(',')+':'+intersections.join(',');")
            .unwrap(),
        JsValue::string("50:true")
    );
    r.viewport(GeometryRect {
        y: 200.0,
        ..viewport
    })
    .unwrap();
    assert_eq!(
        r.eval("sizes.length+':'+intersections.join(',');").unwrap(),
        JsValue::string("1:true,false")
    );
}
#[test]
fn canvas_pixels_transform_paths_and_resize_reset_use_immutable_snapshots() {
    let mut r = runtime();
    r.eval("var canvas=document.getElementById('chart');var c=canvas.getContext('2d');c.fillStyle='#ff0000';c.fillRect(0,0,20,20);c.save();c.translate(30,0);c.fillStyle='blue';c.fillRect(0,0,10,10);c.restore();c.beginPath();c.moveTo(40,30);c.lineTo(60,30);c.lineTo(50,45);c.closePath();c.fill();").unwrap();
    assert_eq!(
        r.eval(
            "c.getImageData(1,1,1,1).data.join(',')+':'+c.getImageData(31,1,1,1).data.join(',');"
        )
        .unwrap(),
        JsValue::string("255,0,0,255:0,0,255,255")
    );
    let snapshot = r.document();
    let node = snapshot.get_element_by_id("chart").unwrap();
    let before = snapshot.canvas_bitmap(node).unwrap().clone();
    r.eval("canvas.width=canvas.width;").unwrap();
    assert_eq!(
        r.eval("c.fillStyle+':'+c.getImageData(1,1,1,1).data[3];")
            .unwrap(),
        JsValue::string("#000000:0")
    );
    assert_eq!(before.pixels[3], 255);
    assert_ne!(before.id, r.document().canvas_bitmap(node).unwrap().id);
}
#[test]
fn cross_origin_draw_image_taints_readback_and_resize_cleans_it() {
    let mut r = runtime();
    let image = r.document().get_element_by_id("source").unwrap();
    r.image(
        image,
        &"https://example.test/image.png".parse().unwrap(),
        CanvasImage {
            width: 1,
            height: 1,
            pixels: Arc::from([0, 255, 0, 255]),
            tainted: true,
        },
    )
    .unwrap();
    r.eval("var canvas=document.getElementById('chart');var c=canvas.getContext('2d');c.drawImage(document.getElementById('source'),0,0);var blocked=false;try{c.getImageData(0,0,1,1);}catch(e){blocked=e.name==='SecurityError';}").unwrap();
    assert_eq!(r.eval("blocked;").unwrap(), JsValue::Bool(true));
    r.eval("canvas.setAttribute('width','100');c.getImageData(0,0,1,1);")
        .unwrap();
}
#[test]
fn canvas_text_uses_system_shaping_and_draws_nonempty_glyphs() {
    let mut r = runtime();
    r.eval("var c=document.getElementById('chart').getContext('2d');c.font='18px sans-serif';c.fillStyle='black';c.fillText('Chart',2,25);var width=c.measureText('Chart').width;").unwrap();
    assert!(matches!(r.eval("width;").unwrap(),JsValue::Number(n)if n>20.0));
    let d = r.document();
    let bitmap = d
        .canvas_bitmap(d.get_element_by_id("chart").unwrap())
        .unwrap();
    assert!(bitmap.pixels.chunks_exact(4).filter(|p| p[3] > 0).count() > 50);
}
