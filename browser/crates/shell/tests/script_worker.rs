//! Exercise the real persistent worker, including canceled resource futures.
use net::{Client, Fetched};
use shell::{
    fetcher::FetchEvent,
    script_worker::{DomCommand, ScriptWorker},
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn fetched(url: &str, markup: &str) -> Fetched {
    Fetched {
        url: url.parse().unwrap(),
        status: 200,
        content_type: Some("text/html".into()),
        script_policies: Vec::new(),
        bytes: markup.as_bytes().to_vec(),
    }
}
fn receive(rx: &mpsc::Receiver<FetchEvent>, predicate: impl Fn(&FetchEvent) -> bool) -> FetchEvent {
    let start = Instant::now();
    loop {
        let event = rx
            .recv_timeout(Duration::from_secs(3).saturating_sub(start.elapsed()))
            .unwrap();
        if predicate(&event) {
            return event;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canceled_stylesheet_load_does_not_delay_the_next_navigation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (connected, connection) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..size]).contains("/slow.css"));
        connected.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type:text/css\r\nContent-Length:0\r\n\r\n")
            .await;
    });
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(AtomicU64::new(1));
    let mut worker = ScriptWorker::spawn(
        Arc::new(Client::new(Default::default()).unwrap()),
        tx,
        Arc::new(|| {}),
        active.clone(),
    )
    .unwrap();
    worker.initialize(
        1,
        fetched(&url, "<link rel=stylesheet href='/slow.css'><p>old</p>"),
        Default::default(),
    );
    connection.await.unwrap();
    let start = Instant::now();
    active.store(2, Ordering::Relaxed);
    worker.initialize(2, fetched(&url, "<p id=x>new</p>"), Default::default());
    receive(&rx, |e| matches!(e, FetchEvent::Done { id: 2, .. }));
    assert!(start.elapsed() < Duration::from_millis(500));
    worker.shutdown();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timer_mutations_publish_resources_and_initial_focus_console() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..size]).contains("/timer.png"));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type:image/png\r\nContent-Length:3\r\nConnection:close\r\n\r\nbad").await.unwrap();
    });
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(AtomicU64::new(1));
    let mut worker = ScriptWorker::spawn(
        Arc::new(Client::new(Default::default()).unwrap()),
        tx,
        Arc::new(|| {}),
        active,
    )
    .unwrap();
    worker.initialize(1, fetched(&url, "<input id=x><button id=spin>spin</button><div id=root></div><script>document.getElementById('x').focus();console.log('local');document.getElementById('spin').onclick=()=>{while(true){}};setTimeout(()=>{document.getElementById('root').innerHTML='<img src=timer.png>';},30);</script>"), Default::default());
    let initial = receive(&rx, |e| matches!(e, FetchEvent::ScriptPage { .. }));
    let FetchEvent::ScriptPage {
        page,
        focused,
        console,
        ..
    } = initial
    else {
        unreachable!()
    };
    assert_eq!(focused, page.document.get_element_by_id("x"));
    assert_eq!(console, vec!["local"]);
    receive(
        &rx,
        |e| matches!(e, FetchEvent::Image { url, .. } if url.path() == "/timer.png"),
    );
    server.await.unwrap();
    worker.event(
        1,
        DomCommand::Click(page.document.get_element_by_id("spin").unwrap()),
    );
    receive(&rx, |e| {
        matches!(e, FetchEvent::ScriptError { stopped: true, .. })
    });
    worker.shutdown();
}
#[test]
fn tab_input_keyup_and_button_activation_use_the_persistent_realm() {
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(AtomicU64::new(1));
    let mut worker = ScriptWorker::spawn(
        Arc::new(Client::new(Default::default()).unwrap()),
        tx,
        Arc::new(|| {}),
        active,
    )
    .unwrap();
    worker.initialize(1, fetched("http://localhost/", "<input id=x><button id=b type=button>go</button><output id=out>0</output><script>let n=0;document.getElementById('b').onclick=()=>document.getElementById('out').textContent=++n;document.getElementById('x').onkeyup=()=>document.getElementById('out').textContent='up';</script>"), Default::default());
    let FetchEvent::ScriptPage { page, .. } =
        receive(&rx, |e| matches!(e, FetchEvent::ScriptPage { .. }))
    else {
        unreachable!()
    };
    let input = page.document.get_element_by_id("x").unwrap();
    let button = page.document.get_element_by_id("b").unwrap();
    worker.event(1, DomCommand::Tab { reverse: false });
    receive(
        &rx,
        |e| matches!(e, FetchEvent::ScriptPage { focused: Some(node), .. } if *node == input),
    );
    worker.event(
        1,
        DomCommand::Key {
            node: None,
            key: "A".into(),
            text: Some("A".into()),
            pressed: true,
        },
    );
    receive(
        &rx,
        |e| matches!(e, FetchEvent::ScriptPage { page, .. } if page.document.control_value(input) == "A"),
    );
    worker.event(
        1,
        DomCommand::Key {
            node: None,
            key: "A".into(),
            text: None,
            pressed: false,
        },
    );
    receive(
        &rx,
        |e| matches!(e, FetchEvent::ScriptPage { page, .. } if page.document.text_content(page.document.get_element_by_id("out").unwrap()) == "up"),
    );
    worker.event(1, DomCommand::Tab { reverse: false });
    receive(
        &rx,
        |e| matches!(e, FetchEvent::ScriptPage { focused: Some(node), .. } if *node == button),
    );
    worker.event(
        1,
        DomCommand::Key {
            node: None,
            key: "Enter".into(),
            text: None,
            pressed: true,
        },
    );
    receive(
        &rx,
        |e| matches!(e, FetchEvent::ScriptPage { page, .. } if page.document.text_content(page.document.get_element_by_id("out").unwrap()) == "1"),
    );
    worker.event(1, DomCommand::Click(input));
    worker.event(
        1,
        DomCommand::Key {
            node: None,
            key: "B".into(),
            text: Some("B".into()),
            pressed: true,
        },
    );
    receive(
        &rx,
        |e| matches!(e, FetchEvent::ScriptPage { page, .. } if page.document.control_value(input) == "AB"),
    );
    worker.shutdown();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_completion_updates_the_live_page_and_publishes_same_document_history() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let amount = stream.read(&mut request).await.unwrap();
        assert!(amount > 0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type:application/json\r\nContent-Length:12\r\nConnection:close\r\n\r\n{\"value\":42}").await.unwrap();
    });
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(AtomicU64::new(1));
    let mut worker = ScriptWorker::spawn(
        Arc::new(Client::new(Default::default()).unwrap()),
        tx,
        Arc::new(|| {}),
        active,
    )
    .unwrap();
    worker.initialize(1,fetched(&url,"<output id=result>waiting</output><script>fetch('/api').then(r=>r.json()).then(data=>{document.getElementById('result').textContent=data.value;history.pushState({page:2},'', '/view');});</script>"),Default::default());
    receive(
        &rx,
        |e| matches!(e,FetchEvent::SameDocument{url,back:true,..}if url.path()=="/view"),
    );
    receive(
        &rx,
        |e| matches!(e,FetchEvent::ScriptPage{page,..}if page.document.text_content(page.document.get_element_by_id("result").unwrap())=="42"),
    );
    worker.event(1, DomCommand::HistoryGo(-1));
    receive(
        &rx,
        |e| matches!(e,FetchEvent::SameDocument{url,forward:true,..}if url.path()=="/"),
    );
    worker.shutdown();
    server.await.unwrap();
}
#[test]
fn native_prompt_and_permission_response_do_not_need_javascript_on_ui_thread() {
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(AtomicU64::new(1));
    let mut worker = ScriptWorker::spawn(
        Arc::new(Client::new(Default::default()).unwrap()),
        tx,
        Arc::new(|| {}),
        active,
    )
    .unwrap();
    worker.initialize(1,fetched("https://example.test/","<button id=b>ask</button><output id=result>before</output><script>var answer=prompt('Name?','default');document.getElementById('result').textContent=answer;document.getElementById('b').onclick=()=>Notification.requestPermission().then(permission=>document.getElementById('result').textContent=permission);</script>"),Default::default());
    let FetchEvent::Dialog { request, reply, .. } =
        receive(&rx, |e| matches!(e, FetchEvent::Dialog { .. }))
    else {
        unreachable!()
    };
    assert_eq!(request.origin, "https://example.test");
    reply.send(Some("native".into())).unwrap();
    let FetchEvent::ScriptPage { page, .. } =
        receive(&rx, |e| matches!(e, FetchEvent::ScriptPage { .. }))
    else {
        unreachable!()
    };
    assert_eq!(
        page.document
            .text_content(page.document.get_element_by_id("result").unwrap()),
        "native"
    );
    worker.event(
        1,
        DomCommand::Click(page.document.get_element_by_id("b").unwrap()),
    );
    let FetchEvent::WebRequest {
        command: web_api::WebCommand::Permission { id, name, gesture },
        ..
    } = receive(&rx, |e| matches!(e, FetchEvent::WebRequest { .. }))
    else {
        unreachable!()
    };
    assert!(gesture);
    assert_eq!(name, "notifications");
    worker.decision(
        1,
        id,
        name,
        "once".into(),
        shell::web_ui::Action::Permission,
    );
    receive(
        &rx,
        |e| matches!(e,FetchEvent::ScriptPage{page,..}if page.document.text_content(page.document.get_element_by_id("result").unwrap())=="granted"),
    );
    worker.shutdown();
}
