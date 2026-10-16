//! Real loopback transport: CORS gates run before a disallowed redirect dial.
use dom_bindings::{html, js::JsValue, policy::Policy};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use web_api::{
    headers::Headers,
    http::{self, Credentials, Mode, Request},
    policy::Csp,
    services::Services,
    PageRuntime, RuntimeConfig,
};

async fn server() -> (
    url::Url,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let requests = log.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let log = requests.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..n]);
                    if data.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    if data.len() > 32768 {
                        return;
                    }
                }
                let request = String::from_utf8_lossy(&data).into_owned();
                log.lock().unwrap().push(request.clone());
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let origin = request
                    .lines()
                    .find_map(|s| s.strip_prefix("origin: "))
                    .unwrap_or("");
                let mut headers = String::new();
                if path != "/deny" {
                    headers.push_str(&format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: x-token, content-type\r\nAccess-Control-Allow-Credentials: true\r\n"));
                }
                let (status,mime,body)=match path{
            "/worker.js"=>("200 OK","text/javascript","onmessage=e=>{postMessage({value:e.data.value+1,dom:typeof document});close();};"),
            "/events"=>("200 OK","text/event-stream","\u{feff}id: 7\r\nevent: update\r\ndata: first\r\ndata: second\r\n\r\n"),
            "/login"=>{headers.push_str("Set-Cookie: session=secret; HttpOnly; Path=/\r\n");("200 OK","application/json","{\"loggedIn\":true}")},
            "/redirect"=>{headers.push_str("Location: http://localhost:9/forbidden\r\n");("302 Found","text/plain","")},
            _=>("200 OK","application/json","{\"value\":42}"),
        };
                let response=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, log, task)
}
#[tokio::test]
async fn preflight_credentials_exposure_and_rejection() {
    let (url, log, server) = server().await;
    let client = net::Client::new(Default::default()).unwrap();
    let document = "http://origin.test/".parse().unwrap();
    let mut request = Request::get(url.join("api").unwrap());
    request.method = "PUT".into();
    request.headers = Headers(vec![("x-token".into(), "one".into())]);
    request.credentials = Credentials::Include;
    let response = http::fetch(&client, &document, &Csp::default(), request)
        .await
        .unwrap();
    assert_eq!(response.kind, "cors");
    assert_eq!(response.status, 200);
    let requests = log.lock().unwrap().clone();
    assert!(requests[0].starts_with("OPTIONS"));
    assert!(requests[0].contains("access-control-request-headers: x-token"));
    assert!(requests[1].starts_with("PUT"));
    assert!(http::fetch(
        &client,
        &document,
        &Csp::default(),
        Request::get(url.join("deny").unwrap())
    )
    .await
    .is_err());
    let mut opaque = Request::get(url.join("deny").unwrap());
    opaque.mode = Mode::NoCors;
    let response = http::fetch(&client, &document, &Csp::default(), opaque)
        .await
        .unwrap();
    assert_eq!(response.status, 0);
    assert!(response.body.is_empty());
    server.abort();
}
#[tokio::test]
async fn login_cookie_shared_with_document_and_redirect_policy_precedes_connection() {
    let (url, log, server) = server().await;
    let client = net::Client::new(Default::default()).unwrap();
    let response = http::fetch(
        &client,
        &url,
        &Csp::default(),
        Request::get(url.join("login").unwrap()),
    )
    .await
    .unwrap();
    assert!(response.headers.get("set-cookie").is_none());
    assert_eq!(client.cookies.header(&url, &url, false), "session=secret");
    assert!(client.cookies.header(&url, &url, true).is_empty());
    http::fetch(
        &client,
        &url,
        &Csp::default(),
        Request::get(url.join("me").unwrap()),
    )
    .await
    .unwrap();
    assert!(log
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .contains("cookie: session=secret"));
    let policy = Csp::parse(&["connect-src 'self'".into()]);
    let error = http::fetch(
        &client,
        &url,
        &policy,
        Request::get(url.join("redirect").unwrap()),
    )
    .await
    .unwrap_err();
    assert!(error.message.contains("CSP"));
    server.abort();
}
fn runtime(url: url::Url) -> PageRuntime {
    let doc = html::parse_full(
        b"<output id=result>before</output>",
        &url,
        Default::default(),
    );
    PageRuntime::new(
        doc,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        RuntimeConfig::default(),
    )
    .unwrap()
}
async fn drive(runtime: &mut PageRuntime, services: &mut Services, done: &str) {
    for _ in 0..600 {
        for command in runtime.commands() {
            assert!(services.dispatch(command).unwrap().is_none());
        }
        while let Some(completion) = services.poll() {
            runtime.complete(completion).unwrap();
        }
        if runtime.eval(done).unwrap().truthy() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("service did not complete: {:?}", runtime.console());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dedicated_worker_has_no_dom_and_messages_survive_gc() {
    let (url, _, server) = server().await;
    let client = Arc::new(net::Client::new(Default::default()).unwrap());
    let mut r = runtime(url);
    let mut services = Services::new(client, tokio::runtime::Handle::current());
    r.eval("var received=null;var worker=new Worker('/worker.js');worker.onmessage=e=>received=e.data;worker.postMessage({value:41});").unwrap();
    r.gc_collect();
    drive(&mut r, &mut services, "received!==null;").await;
    assert_eq!(
        r.eval("received.value+':'+received.dom;").unwrap(),
        JsValue::string("42:undefined")
    );
    services.cancel();
    server.abort();
}
#[tokio::test]
async fn event_source_streams_named_multiline_events_and_close_cancels() {
    let (url, _, server) = server().await;
    let mut r = runtime(url);
    let mut services = Services::new(
        Arc::new(net::Client::new(Default::default()).unwrap()),
        tokio::runtime::Handle::current(),
    );
    r.eval("var received='';var source=new EventSource('/events');source.addEventListener('update',e=>{received=e.data+':'+e.lastEventId;source.close();});").unwrap();
    drive(&mut r, &mut services, "received!=='';").await;
    assert_eq!(
        r.eval("received;").unwrap(),
        JsValue::string("first\nsecond:7")
    );
    for command in r.commands() {
        services.dispatch(command).unwrap();
    }
    assert!(services.is_idle());
    server.abort();
}
#[tokio::test]
#[allow(clippy::result_large_err)] // The auxiliary handshake fixes this callback's error type.
async fn websocket_echo_uses_origin_and_buffered_lifecycle() {
    use futures::{SinkExt, StreamExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let expected = url.origin().ascii_serialization();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                  response| {
                assert_eq!(request.headers().get("origin").unwrap(), expected.as_str());
                Ok(response)
            },
        )
        .await
        .unwrap();
        let message = socket.next().await.unwrap().unwrap();
        socket.send(message).await.unwrap();
        let _ = socket.next().await;
    });
    let mut r = runtime(url);
    let mut services = Services::new(
        Arc::new(net::Client::new(Default::default()).unwrap()),
        tokio::runtime::Handle::current(),
    );
    r.eval("var received='';var socket=new WebSocket(location.href);socket.onopen=()=>socket.send('hello');socket.onmessage=e=>{received=e.data;socket.close(1000,'done');};").unwrap();
    drive(&mut r, &mut services, "received==='hello';").await;
    drive(&mut r, &mut services, "socket.readyState===3;").await;
    assert_eq!(
        r.eval("socket.bufferedAmount;").unwrap(),
        JsValue::Number(0.0)
    );
    services.cancel();
    server.await.unwrap();
}
#[tokio::test]
async fn idle_page_services_make_zero_startup_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let mut r = runtime(url);
    let mut services = Services::new(
        Arc::new(net::Client::new(Default::default()).unwrap()),
        tokio::runtime::Handle::current(),
    );
    r.eval("navigator.onLine;").unwrap();
    r.poll_timers().unwrap();
    assert!(r.commands().is_empty());
    assert!(services.poll().is_none());
    assert!(services.is_idle());
    assert!(
        tokio::time::timeout(Duration::from_millis(80), listener.accept())
            .await
            .is_err()
    );
}
