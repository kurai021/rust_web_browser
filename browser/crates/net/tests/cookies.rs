use net::cookies::CookieJar;
#[test]
fn cookie_policy_enforces_domain_path_secure_http_only_and_site() {
    let jar = CookieJar::default();
    let url = "https://shop.example.com/account/login".parse().unwrap();
    let foreign = "https://tracker.test/".parse().unwrap();
    assert!(jar.set(
        &url,
        &url,
        "session=secret; Secure; HttpOnly; Path=/account",
        false
    ));
    assert!(jar.header(&url, &url, true).is_empty());
    assert_eq!(jar.header(&url, &url, false), "session=secret");
    assert!(!jar.set(&url, &url, "session=overwritten; Path=/account", true));
    assert!(!jar.set(&url, &url, "bad=1; Domain=com", false));
    assert!(!jar.set(&url, &url, "bad=1; Domain=other.test", false));
    assert!(!jar.set(&url, &url, "bad=1; SameSite=None", false));
    assert!(jar.header(&url, &foreign, false).is_empty());
    assert!(!jar.set(&url, &foreign, "third=1", false));
    assert!(jar
        .header(&url.join("/public").unwrap(), &url, false)
        .is_empty());
    assert!(!jar.set(
        &"http://shop.example.com/account/login".parse().unwrap(),
        &"http://shop.example.com/".parse().unwrap(),
        "session=plain; Path=/account",
        false
    ));
    assert!(!jar.set(
        &url,
        &url,
        "__Host-x=1; Secure; Domain=example.com; Path=/",
        false
    ));
}
