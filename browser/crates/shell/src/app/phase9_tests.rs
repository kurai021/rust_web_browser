use super::browser_ui::{Action, Field};
use super::*;
fn blank() -> ShellApp {
    let shared = crate::fetcher::Shared::open(Default::default(), None).unwrap();
    let wake = Arc::new(|| {});
    let fetcher = Fetcher::spawn_shared(shared, wake.clone()).unwrap();
    let mut app = ShellApp::new(fetcher, None, false, None);
    app.configure_rendering(true, false, wake);
    app.initialize_navigation();
    app
}
#[test]
fn trusted_settings_apply_cookie_policy_without_opening_a_connection() {
    let mut app = blank();
    app.navigate("about:preferences");
    app.chrome_action(Action::Edit(Field::Homepage));
    app.browser.edit.set_text("about:bookmarks");
    app.chrome_action(Action::Commit);
    assert_eq!(app.profile.preferences.general.homepage, "about:bookmarks");
    let original = app.profile.preferences.general.search_engine.clone();
    app.chrome_action(Action::Edit(Field::Search));
    app.browser.edit.set_text("file:///{query}");
    app.chrome_action(Action::Commit);
    assert_eq!(app.profile.preferences.general.search_engine, original);
    app.chrome_action(Action::Dismiss);
    app.chrome_action(Action::Toggle(5));
    let third = "https://third.test/".parse().unwrap();
    let first = "https://first.test/".parse().unwrap();
    assert!(app.shared.client.cookies.set(
        &third,
        &first,
        "own-test=1; Secure; SameSite=None",
        false
    ));
    assert!(app
        .shared
        .client
        .cookies
        .header(&third, &first, false)
        .contains("own-test=1"));
    app.chrome_action(Action::Toggle(5));
    assert!(app
        .shared
        .client
        .cookies
        .header(&third, &first, false)
        .is_empty());
    assert_eq!(app.shared.client.network_attempts(), 0);
    app.shutdown();
}
#[test]
fn web_font_aliases_are_tab_local_and_never_enter_the_browser_chrome_database() {
    let mut app = blank();
    let face = app.font_system.db().faces().next().unwrap().id;
    let data = app
        .font_system
        .db()
        .with_face_data(face, |bytes, _| bytes.to_vec())
        .unwrap();
    let url: Url = "http://127.0.0.1/".parse().unwrap();
    let head = net::ResponseHead {
        url: url.clone(),
        status: 200,
        content_type: Some("text/html".into()),
        script_policies: Vec::new(),
    };
    let doc = html::parse_full(b"<p>font scope</p>", &url, Default::default());
    let page = Arc::new(Page::preview(&head, doc));
    app.page = Some(page.clone());
    let alias = crate::fonts::web_family("Phase9ScopedFont");
    app.on_fetch_event(FetchEvent::Fonts {
        id: app.navigation_id,
        page,
        assets: vec![crate::fonts::FontAsset {
            family: "Phase9ScopedFont".into(),
            weight: 400,
            italic: false,
            data: Arc::new(data),
        }],
    });
    assert!(app
        .page_font_db
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == &alias)));
    assert!(!app
        .font_system
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == &alias)));
    app.new_tab(None).unwrap();
    assert!(!app
        .page_font_db
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == &alias)));
    app.select_tab(0);
    assert!(app
        .page_font_db
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == &alias)));
    app.shutdown();
}
