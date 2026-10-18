use profile::{Data, Preferences, SavedTab, Store};
#[test]
fn profile_round_trip_preserves_session_settings_bookmarks_and_private_permissions() {
    let directory = std::env::temp_dir().join(format!(
        "browser-profile-{}-{}",
        std::process::id(),
        profile::now()
    ));
    let url = "https://user:password@example.test/article"
        .parse()
        .unwrap();
    {
        let mut store = Store::open(Some(directory.clone())).unwrap();
        store.data.visit(&url, "An article");
        store.data.bookmark(&url, "Read later");
        store.data.session.tabs = vec![SavedTab {
            current: url.to_string(),
            back: vec!["about:blank".into()],
            scroll_y: 321.0,
            ..Default::default()
        }];
        store.preferences.general.homepage = "about:bookmarks".into();
        store
            .preferences
            .privacy
            .site_javascript
            .insert("https://example.test".into(), false);
        store.changed();
        store.shutdown();
        assert!(store.take_error().is_none());
    }
    let mut loaded = Store::open(Some(directory.clone())).unwrap();
    assert!(!std::fs::read_to_string(directory.join("browser.json"))
        .unwrap()
        .contains("password"));
    assert_eq!(
        loaded.data.history[0].url.as_str(),
        "https://example.test/article"
    );
    assert_eq!(
        loaded.data.session.tabs[0].current,
        "https://example.test/article"
    );
    assert_eq!(loaded.data.session.tabs[0].scroll_y, 321.0);
    assert!(!loaded.preferences.javascript(&url));
    assert_eq!(loaded.preferences.general.homepage, "about:bookmarks");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(directory.join("browser.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    loaded.shutdown();
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn local_suggestions_and_history_deletion_respect_site_and_time() {
    let mut data = Data::default();
    data.visit(&"https://first.test/a".parse().unwrap(), "Rust handbook");
    data.visit(&"https://second.test/b".parse().unwrap(), "Other page");
    data.bookmark(&"https://first.test/a".parse().unwrap(), "Rust bookmark");
    assert_eq!(data.suggestions("rust", 10).len(), 1);
    data.history[0].time = 10;
    data.history[1].time = 20;
    data.delete_history(15, Some("first.test"));
    assert_eq!(data.history.len(), 2);
    data.delete_history(0, Some("first.test"));
    assert_eq!(data.history.len(), 1);
    assert_eq!(data.history[0].url.host_str(), Some("second.test"));
    data.delete_history(0, None);
    assert!(data.history.is_empty());
    assert_eq!(data.bookmarks.len(), 1);
}
#[test]
fn search_encoding_cannot_inject_parameters_and_invalid_settings_fail() {
    let mut prefs = Preferences::default();
    let url = prefs.search_url("a & admin=true / café").unwrap();
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        vec![("q".into(), "a & admin=true / café".into())]
    );
    prefs.general.search_engine = "file:///{query}".into();
    assert!(prefs.validate().is_err());
}
