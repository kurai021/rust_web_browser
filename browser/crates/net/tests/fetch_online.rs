//! Online `net` tests: real internet (run with `cargo test -- --ignored`).
//!
//! Validates the Phase 1 exit criteria end to end: real HTTPS fetch,
//! redirect following, and TLS error mapping against badssl.com.

use std::time::Duration;

use net::{Client, Error, FetchOptions, Progress, TlsError};

fn client() -> Client {
    Client::new(FetchOptions::default()).unwrap()
}

fn no_progress(_: Progress) {}

#[tokio::test]
#[ignore]
async fn fetches_example_com() {
    let url = "https://example.com/".parse().unwrap();
    let fetched = client().fetch(&url, &mut no_progress).await.unwrap();
    assert_eq!(fetched.status, 200);
    assert!(fetched
        .content_type
        .as_deref()
        .unwrap_or("")
        .contains("text/html"));
    let text = String::from_utf8_lossy(&fetched.bytes);
    assert!(text.contains("Example Domain"));
}

#[tokio::test]
#[ignore]
async fn expired_cert_maps() {
    let url = "https://expired.badssl.com/".parse().unwrap();
    assert_eq!(
        client().fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::Tls(TlsError::Expired)
    );
}

#[tokio::test]
#[ignore]
async fn wrong_host_maps() {
    let url = "https://wrong.host.badssl.com/".parse().unwrap();
    assert_eq!(
        client().fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::Tls(TlsError::WrongHost)
    );
}

#[tokio::test]
#[ignore]
async fn self_signed_maps_to_unknown_issuer() {
    let url = "https://self-signed.badssl.com/".parse().unwrap();
    assert_eq!(
        client().fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::Tls(TlsError::UnknownIssuer)
    );
}

#[tokio::test]
#[ignore]
async fn http_to_https_redirect_followed() {
    let client = Client::new(FetchOptions {
        total_timeout: Duration::from_secs(30),
        ..FetchOptions::default()
    })
    .unwrap();
    // github.com answers http:// with a 301 to https://.
    let url = "http://github.com/".parse().unwrap();
    let fetched = client.fetch(&url, &mut no_progress).await.unwrap();
    assert_eq!(fetched.url.scheme(), "https");
    assert_eq!(fetched.status, 200);
}
