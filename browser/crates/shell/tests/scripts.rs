//! Blocking/defer parser visibility, persistent DOM and script policy.
use net::{Client, FetchOptions, Fetched};
use shell::scripts::load_scripts;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn blocking_defer_dom_events_and_timers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await.unwrap();
                let body="order.push(document.getElementById('tail').textContent);document.getElementById('counter').addEventListener('click',()=>document.getElementById('out').textContent=++count);setTimeout(()=>document.getElementById('timer').textContent='ready',0);";
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type:text/javascript\r\nContent-Length:{}\r\nConnection:close\r\n\r\n{body}",body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    let url = format!("http://{address}/").parse().unwrap();
    let fetched=Fetched{url,status:200,content_type:Some("text/html".into()),script_policies:Vec::new(),bytes:b"<button id=counter>add</button><output id=out>0</output><p id=timer>waiting</p><script>var order=[];var count=0;order.push(document.getElementById('tail')===null);</script><script defer src='/defer.js'></script><p id=tail>tail</p>".to_vec()};
    let client = Client::new(FetchOptions::default()).unwrap();
    let mut loaded = load_scripts(&client, fetched, Default::default(), None)
        .await
        .unwrap();
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert_eq!(
        loaded.runtime.eval("order.join(',');").unwrap(),
        js::JsValue::string("true,tail")
    );
    let button = loaded
        .runtime
        .document()
        .get_element_by_id("counter")
        .unwrap();
    loaded.runtime.dispatch(button, "click", "").unwrap();
    let doc = loaded.runtime.document();
    assert_eq!(doc.text_content(doc.get_element_by_id("out").unwrap()), "1");
    loaded.runtime.poll_timers().unwrap();
    let doc = loaded.runtime.document();
    assert_eq!(
        doc.text_content(doc.get_element_by_id("timer").unwrap()),
        "ready"
    );
    server.abort();
}
#[tokio::test]
async fn csp_controls_inline_nonce_and_eval() {
    let url = "https://example.test/".parse().unwrap();
    let fetched=Fetched{url,status:200,content_type:Some("text/html".into()),script_policies:vec!["script-src 'nonce-ok'".into()],bytes:b"<p id=x>before</p><script>document.getElementById('x').textContent='bad';</script><script nonce=ok>document.getElementById('x').textContent='allowed';try{eval('1')}catch(e){document.getElementById('x').textContent+=':blocked';}</script>".to_vec()};
    let client = Client::new(Default::default()).unwrap();
    let loaded = load_scripts(&client, fetched, Default::default(), None)
        .await
        .unwrap();
    assert_eq!(
        loaded
            .page
            .document
            .text_content(loaded.page.document.get_element_by_id("x").unwrap()),
        "allowed:blocked"
    );
    assert!(loaded.errors.iter().any(|e| e.contains("CSP")));
}
#[tokio::test]
async fn csp_rejects_a_redirect_before_connecting_to_its_target() {
    let blocked = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = blocked.local_addr().unwrap();
    let allowed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = allowed.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = allowed.accept().await.unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..size]).contains("/redirect.js"));
        stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation:http://{target}/blocked.js\r\nContent-Length:0\r\nConnection:close\r\n\r\n").as_bytes()).await.unwrap();
    });
    let fetched = Fetched {
        url: format!("http://{origin}/").parse().unwrap(),
        status: 200,
        content_type: Some("text/html".into()),
        script_policies: vec!["script-src 'self'".into()],
        bytes: b"<script src='/redirect.js'></script><p>kept</p>".to_vec(),
    };
    let loaded = load_scripts(
        &Client::new(Default::default()).unwrap(),
        fetched,
        Default::default(),
        None,
    )
    .await
    .unwrap();
    assert!(!loaded.errors.is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), blocked.accept())
            .await
            .is_err()
    );
    server.await.unwrap();
}
