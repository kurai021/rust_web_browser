use web_api::{browsing::History, headers::Headers, policy::Csp, storage::Profile};
#[test]
fn headers_validation_cors_safelists_and_origin_storage() {
    let mut h = Headers::default();
    h.append("X-Test", " a ").unwrap();
    h.append("x-test", "b").unwrap();
    assert_eq!(h.get("X-TEST").unwrap(), "a, b");
    assert!(h.set("x\r\nheader", "bad").is_err());
    assert!(Headers::request_forbidden("Cookie"));
    assert!(Headers::safelisted(
        "content-type",
        "text/plain;charset=UTF-8"
    ));
    assert!(!Headers::safelisted("content-type", "application/json"));
    let a = "https://a.test/".parse().unwrap();
    let b = "https://b.test/".parse().unwrap();
    let p = Profile::default();
    p.storage_set(&a, None, Some("key"), Some("value")).unwrap();
    assert!(p.storage(&b, None).unwrap().is_empty());
    assert!(p
        .storage_set(&a, None, Some("huge"), Some(&"x".repeat(3 * 1024 * 1024)))
        .is_err());
    assert_eq!(p.storage(&a, None).unwrap().get("key").unwrap(), "value");
}
#[test]
fn csp_connect_and_history_are_same_origin_and_mixed_content_gated() {
    let a = "https://example.test/".parse().unwrap();
    let csp = Csp::parse(&["default-src 'none'; connect-src 'self' https://api.test".into()]);
    assert!(csp.allows("connect-src", &a, &a.join("api").unwrap(), None));
    assert!(csp.allows(
        "connect-src",
        &a,
        &"https://api.test/x".parse().unwrap(),
        None
    ));
    assert!(!csp.allows(
        "connect-src",
        &a,
        &"http://example.test/api".parse().unwrap(),
        None
    ));
    let mut h = History::new(a.clone());
    h.push(
        a.join("view").unwrap(),
        serde_json::json!({"page":2}),
        false,
    )
    .unwrap();
    assert_eq!(h.go(-1).unwrap().url, a);
    assert!(h
        .push(
            "https://other.test/".parse().unwrap(),
            serde_json::Value::Null,
            false
        )
        .is_err());
}
