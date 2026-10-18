pub mod support;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_document_history_survives_a_later_document_navigation() {
    let server = support::Server::start().await;
    let shared = Shared::open(Default::default(), None).unwrap();
    let mut browser = app(shared, None, false);
    browser.navigate(server.url("/tab/0").as_str());
    ready(&mut browser, "/tab/0").await;
    let node = browser
        .snapshot()
        .unwrap()
        .document
        .get_element_by_id("route")
        .unwrap();
    browser.click_node(node);
    let start = Instant::now();
    while browser.current_url().and_then(|u| u.fragment()) != Some("one") {
        browser.pump();
        assert!(start.elapsed() < Duration::from_secs(3));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    browser.navigate(server.url("/tab/9").as_str());
    ready(&mut browser, "/tab/9").await;
    browser.back();
    ready(&mut browser, "/tab/0").await;
    assert_eq!(browser.current_url().unwrap().fragment(), Some("one"));
    browser.back();
    ready(&mut browser, "/tab/0").await;
    assert_eq!(browser.current_url().unwrap().fragment(), None);
    browser.shutdown();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_a_committed_preview_never_reactivates_the_previous_document_realm() {
    let server = support::Server::start().await;
    let shared = Shared::open(Default::default(), None).unwrap();
    let mut browser = app(shared, None, false);
    browser.navigate(server.url("/tab/0").as_str());
    ready(&mut browser, "/tab/0").await;
    browser.navigate(server.url("/progressive").as_str());
    let start = Instant::now();
    loop {
        browser.pump();
        if browser
            .snapshot()
            .is_some_and(|p| p.url.path() == "/progressive")
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    browser.stop();
    let start = Instant::now();
    while browser.is_loading() {
        browser.pump();
        assert!(start.elapsed() < Duration::from_secs(3));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(browser.current_url().unwrap().path(), "/progressive");
    assert!(browser
        .snapshot()
        .unwrap()
        .document
        .get_element_by_id("inc")
        .is_none());
    browser.back();
    ready(&mut browser, "/tab/0").await;
    browser.shutdown();
}
use shell::{
    app::ShellApp,
    fetcher::{Fetcher, Shared},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn app(shared: Arc<Shared>, directory: Option<std::path::PathBuf>, restore: bool) -> ShellApp {
    let wake = Arc::new(|| {});
    let fetcher = Fetcher::spawn_shared(shared, wake.clone()).unwrap();
    let mut app = ShellApp::new(fetcher, None, false, None);
    app.configure_profile(profile::Store::open(directory).unwrap(), restore);
    app.configure_rendering(true, false, wake);
    app.initialize_navigation();
    app
}
async fn ready(app: &mut ShellApp, path: &str) {
    let start = Instant::now();
    loop {
        app.pump();
        if !app.is_loading() && app.snapshot().is_some_and(|p| p.url.path() == path) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "tab did not load {path}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clean_blank_startup_and_five_blank_tabs_make_zero_connections() {
    let directory = support::Directory::new();
    let shared = Shared::configured(Default::default(), Some(directory.0.clone()), true).unwrap();
    let mut app = app(shared.clone(), Some(directory.0.clone()), false);
    for _ in 1..5 {
        app.new_tab(None).unwrap();
    }
    for _ in 0..10 {
        app.pump();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    app.navigate("about:preferences");
    app.pump();
    assert_eq!(app.tab_count(), 5);
    assert_eq!(shared.client.network_attempts(), 0);
    assert!(shared.client.request_log().is_empty());
    app.shutdown();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_live_tabs_keep_realms_and_session_storage_separate_and_restore_lazily() {
    let directory = support::Directory::new();
    let server = support::Server::start().await;
    let shared = Shared::open(Default::default(), Some(directory.0.clone())).unwrap();
    let mut browser = app(shared, Some(directory.0.clone()), false);
    for i in 0..5 {
        let url = server.url(&format!("/tab/{i}"));
        if i == 0 {
            browser.navigate(url.as_str());
        } else {
            browser.new_tab(Some(url.as_str())).unwrap();
        }
    }
    for i in 0..5 {
        browser.select_tab(i);
        ready(&mut browser, &format!("/tab/{i}")).await;
    }
    browser.select_tab(0);
    let page = browser.snapshot().unwrap();
    browser.click_node(page.document.get_element_by_id("inc").unwrap());
    let start = Instant::now();
    loop {
        browser.pump();
        let page = browser.snapshot().unwrap();
        if page
            .document
            .text_content(page.document.get_element_by_id("count").unwrap())
            == "0:1:0"
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    browser.select_tab(1);
    assert_eq!(
        browser.snapshot().unwrap().document.text_content(
            browser
                .snapshot()
                .unwrap()
                .document
                .get_element_by_id("count")
                .unwrap()
        ),
        "1:0"
    );
    browser.select_tab(0);
    browser.reorder_tab(0, 4);
    assert_eq!(browser.active_tab(), 4);
    assert_eq!(browser.current_url().unwrap().path(), "/tab/0");
    browser.close_tab(0);
    assert_eq!(browser.tab_count(), 4);
    assert_eq!(browser.current_url().unwrap().path(), "/tab/0");
    browser.shutdown();
    drop(browser);
    let shared = Shared::open(Default::default(), Some(directory.0.clone())).unwrap();
    let mut restored = app(shared.clone(), Some(directory.0.clone()), true);
    ready(&mut restored, "/tab/0").await;
    assert_eq!(restored.tab_count(), 4);
    assert_eq!(
        shared
            .client
            .request_log()
            .iter()
            .filter(|r| r.url.contains("/tab/"))
            .count(),
        1
    );
    restored.select_tab(0);
    ready(&mut restored, "/tab/2").await;
    assert_eq!(restored.current_url().unwrap().path(), "/tab/2");
    restored.shutdown();
}
