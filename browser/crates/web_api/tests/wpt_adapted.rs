use dom_bindings::{html, js::JsValue, policy::Policy};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use web_api::{services::Services, PageRuntime, RuntimeConfig};
fn runtime(url: url::Url) -> PageRuntime {
    let doc = html::parse_full(b"<body></body>", &url, Default::default());
    let mut r = PageRuntime::new(
        doc,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        RuntimeConfig::default(),
    )
    .unwrap();
    r.eval(test_utils::WPT_HARNESS).unwrap();
    r
}
fn report(r: &mut PageRuntime, suite: &str, expected: (usize, usize)) {
    let JsValue::String(json) = r.eval("JSON.stringify(wpt_cases);").unwrap() else {
        panic!("report")
    };
    let report = test_utils::WptReport {
        suite: suite.into(),
        upstream: "wpt@4aa56d3beafc5655fa55491169eb14845b44eab3 (adapted)".into(),
        cases: serde_json::from_str(&json).unwrap(),
    };
    report.publish().unwrap();
    let (pass, fail, skip, _) = report.summary();
    assert_eq!(
        (pass, fail, skip),
        (expected.0, 0, expected.1),
        "{report:#?}"
    );
    assert!(r.console().is_empty(), "{:?}", r.console());
}
#[test]
fn wpt_headers_records_sequences_and_crud() {
    let mut r = runtime("https://example.test/".parse().unwrap());
    r.eval(include_str!("wpt/headers.js")).unwrap();
    report(&mut r, "fetch-headers", (13, 10));
}
async fn run_network(source: &str, suite: &str, counts: (usize, usize)) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let end = loop {
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    if bytes.len() > 2 * 1024 * 1024 {
                        return;
                    }
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if length > 1024 * 1024 {
                    return;
                }
                while bytes.len() < end + length {
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let method = head.split_whitespace().next().unwrap_or("GET");
                let mut response=format!("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\nx-request-method: {method}\r\n");
                for key in [
                    "origin",
                    "user-agent",
                    "accept-charset",
                    "content-length",
                    "content-type",
                ] {
                    if let Some(value) = head.lines().find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case(key))
                            .map(|(_, v)| v.trim())
                    }) {
                        response.push_str(&format!("x-request-{key}: {value}\r\n"));
                    }
                }
                response.push_str("\r\n");
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    let mut r = runtime(url);
    let mut services = Services::new(
        Arc::new(net::Client::new(Default::default()).unwrap()),
        tokio::runtime::Handle::current(),
    );
    r.eval(source).unwrap();
    let start = Instant::now();
    loop {
        for command in r.commands() {
            assert!(services.dispatch(command).unwrap().is_none());
        }
        while let Some(completion) = services.poll() {
            r.complete(completion).unwrap();
        }
        if !r.has_pending() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "WPT timeout");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    report(&mut r, suite, counts);
    services.cancel();
    server.abort();
}
#[tokio::test]
async fn wpt_fetch_request_content_types_origin_and_wire_headers() {
    run_network(
        include_str!("wpt/fetch.js"),
        "fetch-request-headers",
        (18, 7),
    )
    .await;
}
#[tokio::test]
async fn wpt_xhr_method_case_preservation_with_async_transport() {
    run_network(include_str!("wpt/xhr.js"), "xhr-method-case", (9, 0)).await;
}
