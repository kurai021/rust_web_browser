//! End-to-end assertions against the explicit loopback workshop server.
use shell::{scripts, viewport};
use std::{sync::Arc, time::Duration};
fn text(runtime: &web_api::PageRuntime, id: &str) -> String {
    let doc = runtime.document();
    doc.get_element_by_id(id)
        .map_or_else(String::new, |node| doc.text_content(node))
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url: url::Url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8097/".into())
        .parse()?;
    let profile = std::env::args().nth(2).map(std::path::PathBuf::from);
    let client = Arc::new(net::Client::new(Default::default())?);
    let config = scripts::headless_config(&client, profile.clone())?;
    let fetched = client.fetch(&url, &mut |_| {}).await?;
    let mut loaded = scripts::load_scripts_configured(
        &client,
        fetched,
        Default::default(),
        None,
        config.clone(),
    )
    .await?;
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(500)).await?;
    assert_eq!(text(&loaded.runtime, "fetch-state"), "Fetched 4 JSON rows");
    assert_eq!(
        text(&loaded.runtime, "worker-state"),
        "Worker total: 200 · DOM: undefined"
    );
    assert_eq!(
        text(&loaded.runtime, "socket-state"),
        "WebSocket echo: hello from the page"
    );
    assert_eq!(
        text(&loaded.runtime, "sse-state"),
        "EventSource update: ready · id 2"
    );
    assert!(text(&loaded.runtime, "idb-state").starts_with("IndexedDB: saved visits"));
    let page = loaded.snapshot();
    let styles = page.computed_styles(Default::default());
    shell::images::load_images(&client, &page, &styles, |_, _| {}).await;
    loaded.register_images()?;
    let mut fonts = cosmic_text::FontSystem::new();
    let page = loaded.snapshot();
    let styles = page.computed_styles(Default::default());
    let frame =
        viewport::headless_frame(&page, &styles, layout::Size::new(1000.0, 900.0), &mut fonts);
    loaded.runtime.geometry(
        viewport::web_geometry(&frame.layout, &Default::default()),
        viewport::observer_viewport(1000.0, 900.0),
    )?;
    let narrow_styles = page.computed_styles(css::Environment {
        width: 620.0,
        height: 900.0,
        ..Default::default()
    });
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
                .is_some_and(|c| c.split_ascii_whitespace().any(|c| c == "card"))
        {
            assert!(
                b.border_box.x >= 0.0 && b.border_box.x + b.border_box.width <= 620.1,
                "narrow card overflow: {:?}",
                b.border_box
            );
        }
    }
    assert!(text(&loaded.runtime, "resize-state").starts_with("ResizeObserver width: "));
    assert!(
        text(&loaded.runtime, "intersection-state").starts_with("IntersectionObserver visible: ")
    );
    assert!(text(&loaded.runtime, "mutation-state").starts_with("MutationObserver records: "));
    let doc = loaded.runtime.document();
    let chart = doc.get_element_by_id("chart").ok_or("missing chart")?;
    let bitmap = doc.canvas_bitmap(chart).ok_or("missing canvas frame")?;
    assert_eq!((bitmap.width, bitmap.height), (760, 300));
    assert!(
        bitmap
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[2] > 150 && p[0] < 100 && p[3] > 0)
            .count()
            > 1000
    );
    let link = doc
        .get_elements_by_tag_name("a")
        .into_iter()
        .find(|&n| doc.get_attribute(n, "href") == Some("/chart"))
        .ok_or("missing route")?;
    loaded.runtime.begin_gesture();
    loaded.runtime.dispatch(link, "click", "")?;
    loaded.runtime.end_gesture();
    assert_eq!(loaded.runtime.url().path(), "/chart");
    loaded.runtime.history_go(-1)?;
    assert_eq!(
        text(&loaded.runtime, "route-state"),
        format!("Route: {}", url.path())
    );
    let form = doc.get_element_by_id("login-form").ok_or("missing form")?;
    loaded.runtime.begin_gesture();
    loaded.runtime.dispatch(form, "submit", "")?;
    loaded.runtime.end_gesture();
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(250)).await?;
    assert_eq!(text(&loaded.runtime, "login-state"), "Signed in as demo");
    assert!(client
        .cookies
        .header(&url, &url, false)
        .contains("session=demo-session"));
    assert!(!client.cookies.header(&url, &url, true).contains("session="));
    let refresh = doc.get_element_by_id("refresh").ok_or("missing refresh")?;
    loaded.runtime.begin_gesture();
    loaded.runtime.dispatch(refresh, "click", "")?;
    loaded.runtime.end_gesture();
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(100)).await?;
    assert_eq!(
        text(&loaded.runtime, "worker-state"),
        "Worker total: 200 · DOM: undefined"
    );
    assert!(
        loaded.runtime.console().is_empty(),
        "{:?}",
        loaded.runtime.console()
    );
    assert!(loaded.errors.is_empty());
    let visits = text(&loaded.runtime, "storage-state");
    println!("PHASE7 PROBE PASS fetch worker websocket sse idb observers canvas history login refresh paint_hash={:016x} {visits}",frame.hash);
    drop(loaded);
    config.profile.close_session(&config.session)?;
    if let Some(path) = profile {
        client.cookies.save(&path.join("cookies.json"))?;
    }
    Ok(())
}
