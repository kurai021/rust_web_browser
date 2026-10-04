//! Offline `net` fetch tests: local TCP stub servers (no internet).
//!
//! Each test serves canned HTTP/1.1 responses on 127.0.0.1 and asserts
//! product semantics: redirects, caps, gzip, timeouts, HSTS, fallback.

use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use net::{Client, Error, FetchOptions, Progress};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serve `respond` (built from the raw request bytes) for up to `n` connections.
async fn serve<F>(n: usize, respond: F) -> SocketAddr
where
    F: Fn(&[u8], SocketAddr) -> Vec<u8> + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..n {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 16 * 1024];
            let Ok(read) = sock.read(&mut buf).await else {
                continue;
            };
            let response = respond(&buf[..read], addr);
            let _ = sock.write_all(&response).await;
        }
    });
    addr
}

fn http200(body: &[u8], extra: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect()
}

fn test_client(max_body: usize, total_timeout: Duration) -> Client {
    Client::new(FetchOptions {
        max_body_bytes: max_body,
        total_timeout,
        ..FetchOptions::default()
    })
    .unwrap()
}

fn no_progress(_: Progress) {}

#[tokio::test]
async fn get_ok_reports_progress() {
    let addr = serve(1, |_, _| http200(b"hello", "")).await;
    let client = test_client(1024 * 1024, Duration::from_secs(10));
    let url = format!("http://{addr}/").parse().unwrap();
    let mut seen = Vec::new();
    let fetched = client
        .fetch(&url, &mut |p: Progress| seen.push(p.downloaded))
        .await
        .unwrap();
    assert_eq!(fetched.status, 200);
    assert_eq!(fetched.bytes, b"hello");
    assert_eq!(fetched.content_type.as_deref(), Some("text/html"));
    assert_eq!(seen, vec![5]);
}

#[tokio::test]
async fn follows_redirect_chain() {
    let addr = serve(2, |req, addr| {
        if req.starts_with(b"GET /b ") {
            http200(b"final", "")
        } else {
            format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}/b\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
        }
    })
    .await;
    let client = test_client(1024 * 1024, Duration::from_secs(10));
    let url = format!("http://{addr}/a").parse().unwrap();
    let fetched = client.fetch(&url, &mut no_progress).await.unwrap();
    assert_eq!(fetched.bytes, b"final");
    assert!(fetched.url.as_str().ends_with("/b"));
}

#[tokio::test]
async fn too_many_redirects_errors() {
    let addr = serve(4, |req, addr| {
        let path = if req.starts_with(b"GET /2 ") { "/1" } else { "/2" };
        format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}{path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    })
    .await;
    let client = Client::new(FetchOptions {
        max_redirects: 2,
        ..FetchOptions::default()
    })
    .unwrap();
    let url = format!("http://{addr}/1").parse().unwrap();
    assert_eq!(
        client.fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::TooManyRedirects
    );
}

#[tokio::test]
async fn http_error_status_surfaces() {
    let addr = serve(1, |_, _| {
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found"
            .to_vec()
    })
    .await;
    let client = test_client(1024 * 1024, Duration::from_secs(10));
    let url = format!("http://{addr}/missing").parse().unwrap();
    assert_eq!(
        client.fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::HttpStatus(404)
    );
}

#[tokio::test]
async fn body_cap_guards_zip_bombs() {
    let big = vec![b'x'; 64 * 1024];
    let addr = serve(1, move |_, _| http200(&big, "")).await;
    let client = test_client(1024, Duration::from_secs(10));
    let url = format!("http://{addr}/big").parse().unwrap();
    assert_eq!(
        client.fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::BodyTooLarge { cap: 1024 }
    );
}

#[tokio::test]
async fn gzip_is_decoded() {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write as _;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(b"compressed hello").unwrap();
    let gzipped = encoder.finish().unwrap();
    let addr = serve(1, move |_, _| {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            gzipped.len()
        )
        .into_bytes()
        .into_iter()
        .chain(gzipped.iter().copied())
        .collect()
    })
    .await;
    let client = test_client(1024 * 1024, Duration::from_secs(10));
    let url = format!("http://{addr}/").parse().unwrap();
    let fetched = client.fetch(&url, &mut no_progress).await.unwrap();
    assert_eq!(fetched.bytes, b"compressed hello");
}

#[tokio::test]
async fn slow_server_hits_total_timeout() {
    let addr = serve(1, |_, _| {
        std::thread::sleep(Duration::from_millis(400));
        http200(b"late", "")
    })
    .await;
    let client = test_client(1024 * 1024, Duration::from_millis(150));
    let url = format!("http://{addr}/").parse().unwrap();
    assert!(matches!(
        client.fetch(&url, &mut no_progress).await.unwrap_err(),
        Error::Timeout(_)
    ));
}

#[tokio::test]
async fn hsts_upgrades_plain_http() {
    // With an HSTS entry, plain http:// must be upgraded: the plaintext
    // stub server cannot complete TLS, so the fetch must fail (proving
    // the scheme was rewritten) while plain fetch without HSTS succeeds.
    let addr = serve(2, |_, _| http200(b"plain", "")).await;
    let client = test_client(1024 * 1024, Duration::from_secs(10));
    let url = format!("http://{addr}/").parse().unwrap();
    assert!(client.fetch(&url, &mut no_progress).await.is_ok());
    client
        .hsts()
        .note_header("127.0.0.1", "max-age=99999", SystemTime::now());
    let err = client.fetch(&url, &mut no_progress).await.unwrap_err();
    assert!(
        matches!(err, Error::Tls(_) | Error::Connect(_)),
        "expected TLS/connect failure after HSTS upgrade, got: {err}"
    );
}

#[tokio::test]
async fn closed_port_is_a_connect_error() {
    // Bind then drop to get a (very likely) closed port.
    let port = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = test_client(1024 * 1024, Duration::from_secs(5));
    let url = format!("https://127.0.0.1:{port}/").parse().unwrap();
    let err = client
        .fetch_with_http_fallback(&url, true, &mut no_progress)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Connect(_)), "got: {err}");
}
