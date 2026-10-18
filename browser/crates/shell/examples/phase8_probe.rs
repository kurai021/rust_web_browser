//! Real HTTP/worker/module SPA acceptance; automation drives ordinary DOM events.
use shell::{scripts, viewport};
use std::{sync::Arc, time::Duration};
fn text(runtime: &web_api::PageRuntime, id: &str) -> String {
    let doc = runtime.document();
    doc.get_element_by_id(id)
        .map_or_else(String::new, |node| doc.text_content(node))
}
fn click(runtime: &mut web_api::PageRuntime, id: &str) -> Result<(), js::JsError> {
    let node = runtime
        .document()
        .get_element_by_id(id)
        .expect("fixture element");
    runtime.begin_gesture();
    let result = runtime.dispatch(node, "click", "");
    runtime.end_gesture();
    result.map(|_| ())
}
fn route(runtime: &mut web_api::PageRuntime, name: &str) -> Result<(), js::JsError> {
    let doc = runtime.document();
    let node = doc
        .get_elements_by_tag_name("a")
        .into_iter()
        .find(|&node| doc.get_attribute(node, "data-route") == Some(name))
        .expect("route");
    runtime.dispatch(node, "click", "").map(|_| ())
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url: url::Url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8097/modern/".into())
        .parse()?;
    let client = Arc::new(net::Client::new(Default::default())?);
    let fetched = client.fetch(&url, &mut |_| {}).await?;
    let mut loaded = scripts::load_scripts(&client, fetched, Default::default(), None).await?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(500)).await?;
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert!(
        loaded.runtime.console().is_empty(),
        "{:?}",
        loaded.runtime.console()
    );
    assert_eq!(
        text(&loaded.runtime, "summary"),
        "3 tasks · 1 completed · Map snapshot"
    );
    assert_eq!(
        text(&loaded.runtime, "worker-state"),
        "Worker: 2 pending · Map: true · DOM: undefined"
    );
    assert_eq!(
        text(&loaded.runtime, "order-state"),
        "sync → end → promise → observer → microtask → timer"
    );
    assert_eq!(
        text(&loaded.runtime, "frame-state"),
        "Animation frame → microtask checkpoint"
    );
    let input = loaded
        .runtime
        .document()
        .get_element_by_id("task-title")
        .unwrap();
    loaded.runtime.input(input, "Modern async task".into())?;
    let form = loaded
        .runtime
        .document()
        .get_element_by_id("task-form")
        .unwrap();
    loaded.runtime.dispatch(form, "submit", "")?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(200)).await?;
    assert_eq!(
        text(&loaded.runtime, "summary"),
        "4 tasks · 1 completed · Map snapshot"
    );
    assert_eq!(text(&loaded.runtime, "status"), "Task saved through POST");
    let toggle = loaded
        .runtime
        .document()
        .get_elements_by_tag_name("button")
        .into_iter()
        .find(|&node| loaded.runtime.document().text_content(node) == "Complete")
        .unwrap();
    loaded.runtime.dispatch(toggle, "click", "")?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(200)).await?;
    assert_eq!(
        text(&loaded.runtime, "summary"),
        "4 tasks · 2 completed · Map snapshot"
    );
    assert_eq!(text(&loaded.runtime, "revision"), "BigInt revision 3");
    route(&mut loaded.runtime, "completed")?;
    assert_eq!(loaded.runtime.url().path(), "/modern/completed");
    assert_eq!(
        loaded
            .runtime
            .eval("document.querySelectorAll('#tasks li').length;")?,
        js::JsValue::Number(2.0)
    );
    route(&mut loaded.runtime, "about")?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(200)).await?;
    assert!(text(&loaded.runtime, "about").contains("cached namespace"));
    route(&mut loaded.runtime, "all")?;
    route(&mut loaded.runtime, "about")?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(100)).await?;
    assert_eq!(
        loaded.runtime.eval("detailsLoads;")?,
        js::JsValue::Number(1.0)
    );
    loaded.runtime.history_go(-1)?;
    assert_eq!(loaded.runtime.url().path(), "/modern/");
    click(&mut loaded.runtime, "refresh")?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(200)).await?;
    assert_eq!(
        text(&loaded.runtime, "summary"),
        "4 tasks · 2 completed · Map snapshot"
    );
    assert!(
        loaded.runtime.console().is_empty(),
        "{:?}",
        loaded.runtime.console()
    );
    let page = loaded.snapshot();
    let styles = page.computed_styles(Default::default());
    let mut fonts = cosmic_text::FontSystem::new();
    let frame =
        viewport::headless_frame(&page, &styles, layout::Size::new(1000.0, 900.0), &mut fonts);
    let narrow_environment = css::Environment {
        width: 620.0,
        height: 900.0,
        ..Default::default()
    };
    let narrow_styles = page.computed_styles(narrow_environment);
    let narrow = viewport::headless_frame(
        &page,
        &narrow_styles,
        layout::Size::new(620.0, 900.0),
        &mut fonts,
    );
    for b in &narrow.layout.boxes {
        if !b.anonymous
            && page
                .document
                .get_attribute(b.element.node, "class")
                .is_some_and(|value| value.split_ascii_whitespace().any(|class| class == "card"))
        {
            assert!(
                b.border_box.x >= 0.0 && b.border_box.x + b.border_box.width <= 620.1,
                "narrow overflow: {:?}",
                b.border_box
            );
        }
    }
    loaded.runtime.gc_collect();
    println!("PHASE8 PROBE PASS static-modules top-level-await async-fetch POST PATCH generators Proxy Map WeakMap Symbol BigInt structured-clone-worker FIFO rAF dynamic-import-cache history refresh wide narrow paint_hash={:016x}",frame.hash);
    Ok(())
}
