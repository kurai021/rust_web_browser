//! Real transport plus the realm module map: policy/CORS/MIME precede evaluation.
use dom_bindings::{html, js::JsValue, policy::Policy};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use web_api::{modules, policy::Csp, services::Services, PageRuntime, RuntimeConfig};
async fn server(
    redirect: Option<String>,
) -> (
    url::Url,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let log = log.clone();
            let redirect = redirect.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                while data.len() < 32768 {
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..n]);
                    if data.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&data);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                log.lock().unwrap().push(path.clone());
                let mut headers = String::new();
                let mut status = "200 OK";
                let mut mime = "text/javascript";
                let body=match path.as_str(){
            "/entry.js"=>{status="302 Found";headers.push_str("Location: /pkg/main.js\r\n");""},
            "/policy-redirect.js"=>{status="302 Found";headers.push_str(&format!("Location: {}\r\n",redirect.as_deref().unwrap_or("http://localhost:9/blocked")));""},
            "/pkg/main.js"=>"\u{feff}import {value,increment} from './state.js';increment();export const answer=await Promise.resolve(value+1);export const url=import.meta.url;",
            "/pkg/state.js"=>"export let value=40;export function increment(){value++;}",
            "/pkg/extra.js"=>"import {value} from './state.js';export default value+2;",
            "/cors.js"=>{headers.push_str("Access-Control-Allow-Origin: *\r\n");"export const value=7;"},
            "/denied.js"=>"export const value=8;",
            "/bare.js"=>"import 'unmapped-package';",
            "/html.js"=>{mime="text/html";"<p>not javascript</p>"},
            "/slow.js"=>{tokio::time::sleep(Duration::from_secs(2)).await;"globalThis.late=true;export default 1;"},
            _=>"export default 0;",
        };
                let response=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, requests, server)
}
fn runtime(url: url::Url) -> PageRuntime {
    let document = html::parse_full(
        b"<body><output id=result></output>",
        &url,
        Default::default(),
    );
    PageRuntime::new(
        document,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        RuntimeConfig::default(),
    )
    .unwrap()
}
#[tokio::test]
async fn graphs_resolve_against_redirects_and_dynamic_import_deduplicates_inflight_and_loaded_sources(
) {
    let (url, requests, server) = server(None).await;
    let client = Arc::new(net::Client::new(Default::default()).unwrap());
    let policy = Csp::parse(&["script-src 'self'; connect-src 'none'".into()]);
    let graph = modules::load_graph(
        &client,
        url.join("entry.js").unwrap(),
        &url,
        &policy,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(graph.sources.len(), 2);
    let mut r = runtime(url.clone());
    let promise = r.eval_module_graph(&graph).unwrap();
    let ns = r.vm.promise_result(&promise).unwrap().unwrap();
    r.vm.set_global("ns", ns).unwrap();
    assert_eq!(
        r.eval("ns.answer+':'+ns.url;").unwrap(),
        JsValue::string(format!("42:{}pkg/main.js", url))
    );
    r.eval("var one=import('./pkg/extra.js');var two=import('./pkg/extra.js');var distinct=one!==two;var imported='';Promise.all([one,two]).then(values=>imported=values[0].default+':'+(values[0]===values[1])+':'+distinct).catch(error=>imported=error.name+':'+error.message);").unwrap();
    let initial_one = r.vm.get_global("one").unwrap();
    let initial_two = r.vm.get_global("two").unwrap();
    assert!(
        r.vm.is_promise(&initial_one),
        "initial one={initial_one:?}; two={initial_two:?}"
    );
    let mut services = Services::new(client, tokio::runtime::Handle::current());
    for _ in 0..300 {
        for command in r.commands() {
            services.dispatch(command).unwrap();
        }
        while let Some(completion) = services.poll() {
            r.complete(completion).unwrap();
        }
        r.gc_collect();
        if r.eval("imported!=='';").unwrap().truthy() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let one = r.vm.get_global("one").unwrap();
    let two = r.vm.get_global("two").unwrap();
    assert_eq!(
        r.eval("imported;").unwrap(),
        JsValue::string("43:true:true"),
        "one={:?}; two={:?}; console={:?}; requests={:?}",
        r.vm.promise_result(&one),
        r.vm.promise_result(&two),
        r.console(),
        requests.lock().unwrap()
    );
    let log = requests.lock().unwrap().clone();
    assert_eq!(
        log.iter().filter(|s| s.as_str() == "/pkg/extra.js").count(),
        1
    );
    assert_eq!(
        log.iter().filter(|s| s.as_str() == "/pkg/state.js").count(),
        1
    );
    server.abort();
}
#[tokio::test]
async fn cross_origin_modules_require_cors_and_module_mime_is_strict() {
    let (url, _, server) = server(None).await;
    let client = net::Client::new(Default::default()).unwrap();
    let document = "http://origin.test/".parse().unwrap();
    assert!(modules::load_graph(
        &client,
        url.join("cors.js").unwrap(),
        &document,
        &Csp::default(),
        None,
        None
    )
    .await
    .is_ok());
    assert!(modules::load_graph(
        &client,
        url.join("denied.js").unwrap(),
        &document,
        &Csp::default(),
        None,
        None
    )
    .await
    .unwrap_err()
    .message
    .contains("CORS"));
    assert!(modules::load_graph(
        &client,
        url.join("html.js").unwrap(),
        &url,
        &Csp::default(),
        None,
        None
    )
    .await
    .unwrap_err()
    .message
    .contains("MIME"));
    assert!(modules::load_graph(
        &client,
        url.join("bare.js").unwrap(),
        &url,
        &Csp::default(),
        None,
        None
    )
    .await
    .unwrap_err()
    .message
    .contains("bare"));
    server.abort();
}
#[tokio::test]
async fn script_policy_blocks_a_redirect_before_dialing_its_target() {
    let blocked = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}/blocked.js", blocked.local_addr().unwrap());
    let (url, _, server) = server(Some(target)).await;
    let client = net::Client::new(Default::default()).unwrap();
    assert!(modules::load_graph(
        &client,
        url.join("policy-redirect.js").unwrap(),
        &url,
        &Csp::parse(&["script-src 'self'".into()]),
        None,
        None
    )
    .await
    .unwrap_err()
    .message
    .contains("CSP"));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), blocked.accept())
            .await
            .is_err()
    );
    server.abort();
}
#[tokio::test]
async fn cancelled_module_services_never_evaluate_late_sources() {
    let (url, requests, server) = server(None).await;
    let client = Arc::new(net::Client::new(Default::default()).unwrap());
    let mut r = runtime(url);
    let mut services = Services::new(client, tokio::runtime::Handle::current());
    r.eval("var late=false;import('./slow.js').catch(()=>{});")
        .unwrap();
    for command in r.commands() {
        services.dispatch(command).unwrap();
    }
    for _ in 0..100 {
        if !requests.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    services.cancel();
    assert!(services.is_idle());
    assert!(services.poll().is_none());
    assert_eq!(r.eval("late;").unwrap(), JsValue::Bool(false));
    server.abort();
}
