//! Explicit upstream-app acceptance. Application HTML/JS are read verbatim from
//! the pinned external checkout; nothing is rewritten or injected into the app.
use js::JsValue;
use shell::scripts;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn node(runtime: &mut web_api::PageRuntime, selector: &str) -> usize {
    let value = runtime
        .eval(&format!("document.querySelector({selector:?});"))
        .unwrap();
    let id = runtime.vm.host_id(&value).expect("selected Node");
    runtime.handle().node_id(id).unwrap()
}
fn text(runtime: &mut web_api::PageRuntime, selector: &str) -> String {
    let id = node(runtime, selector);
    runtime.document().text_content(id)
}
#[test]
#[ignore = "requires BROWSER_TODOMVC_CHECKOUT at tastejs/todomvc ff43b02e59dfa604386bb382034b2cd07c2bcd8a"]
fn unmodified_vanilla_todomvc_add_toggle_edit_filter_and_remove() {
    std::thread::Builder::new()
        .name("upstream-js-acceptance".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_upstream());
        })
        .unwrap()
        .join()
        .unwrap();
}
async fn run_upstream() {
    let root = PathBuf::from(
        std::env::var_os("BROWSER_TODOMVC_CHECKOUT").expect("explicit upstream fixture checkout"),
    );
    let app = root.join("examples/javascript-es5");
    let dist = root.join("examples/javascript-es6/dist");
    let mut files = HashMap::new();
    files.insert(
        "/index.html".to_owned(),
        ("text/html", std::fs::read(app.join("index.html")).unwrap()),
    );
    for name in [
        "helpers",
        "store",
        "model",
        "template",
        "view",
        "controller",
        "app",
    ] {
        files.insert(
            format!("/src/{name}.js"),
            (
                "text/javascript",
                std::fs::read(app.join(format!("src/{name}.js"))).unwrap(),
            ),
        );
    }
    files.insert(
        "/node_modules/todomvc-common/base.js".into(),
        (
            "text/javascript",
            std::fs::read(dist.join("base.js")).unwrap(),
        ),
    );
    for path in [
        "/node_modules/todomvc-common/base.css",
        "/node_modules/todomvc-app-css/index.css",
    ] {
        files.insert(
            path.into(),
            ("text/css", std::fs::read(dist.join("app.css")).unwrap()),
        );
    }
    let files = Arc::new(files);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("http://{}/index.html", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let files = files.clone();
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
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, mime, body) = files.get(path).map_or(
                    ("404 Not Found", "text/plain", b"not found".as_slice()),
                    |(mime, body)| ("200 OK", *mime, body.as_slice()),
                );
                let header=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body).await;
            });
        }
    });
    let client = Arc::new(net::Client::new(Default::default()).unwrap());
    let fetched = client.fetch(&url, &mut |_| {}).await.unwrap();
    let mut loaded = scripts::load_scripts(&client, fetched, Default::default(), None)
        .await
        .unwrap();
    scripts::settle(client.clone(), &mut loaded, Duration::from_millis(100))
        .await
        .unwrap();
    assert!(
        loaded.errors.is_empty(),
        "loader errors: {:?}",
        loaded.errors
    );
    assert!(
        loaded.runtime.console().is_empty(),
        "console: {:?}",
        loaded.runtime.console()
    );
    for title in ["First & <safe>", "Second task"] {
        let input = node(&mut loaded.runtime, ".new-todo");
        loaded.runtime.input(input, title.to_owned()).unwrap();
        loaded.runtime.dispatch(input, "change", "").unwrap();
    }
    assert_eq!(text(&mut loaded.runtime, ".todo-count"), "2 items left");
    assert_eq!(
        text(&mut loaded.runtime, ".todo-list label"),
        "First & <safe>"
    );
    let toggle = node(&mut loaded.runtime, ".todo-list .toggle");
    loaded.runtime.dispatch(toggle, "click", "").unwrap();
    assert_eq!(text(&mut loaded.runtime, ".todo-count"), "1 item left");
    let label = node(&mut loaded.runtime, ".todo-list li:not(.completed) label");
    loaded.runtime.dispatch(label, "dblclick", "").unwrap();
    let edit = node(&mut loaded.runtime, ".todo-list .edit");
    loaded.runtime.input(edit, "Edited task".into()).unwrap();
    loaded.runtime.dispatch(edit, "keypress", "Enter").unwrap();
    assert_eq!(
        text(&mut loaded.runtime, ".todo-list li:not(.completed) label"),
        "Edited task"
    );
    loaded
        .runtime
        .navigate(url.join("#/completed").unwrap(), false)
        .unwrap();
    assert_eq!(
        loaded
            .runtime
            .eval("document.querySelectorAll('.todo-list li').length;")
            .unwrap(),
        JsValue::Number(1.0)
    );
    assert_eq!(
        text(&mut loaded.runtime, ".todo-list label"),
        "First & <safe>"
    );
    let clear = node(&mut loaded.runtime, ".clear-completed");
    loaded.runtime.dispatch(clear, "click", "").unwrap();
    loaded
        .runtime
        .navigate(url.join("#/").unwrap(), false)
        .unwrap();
    assert_eq!(text(&mut loaded.runtime, ".todo-list label"), "Edited task");
    let remove = node(&mut loaded.runtime, ".todo-list .destroy");
    loaded.runtime.dispatch(remove, "click", "").unwrap();
    loaded.runtime.gc_collect();
    assert_eq!(
        loaded
            .runtime
            .eval("document.querySelectorAll('.todo-list li').length;")
            .unwrap(),
        JsValue::Number(0.0)
    );
    assert!(
        loaded.runtime.console().is_empty(),
        "console: {:?}",
        loaded.runtime.console()
    );
    println!("UPSTREAM TODOMVC PASS add escaped-title toggle edit completed-filter clear-completed all-filter delete; application HTML and seven source files unchanged");
    server.abort();
}
