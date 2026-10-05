//! Deterministic progressive first-paint and deferred-image integration.
use css::Environment;
use shell::fetcher::{Command, FetchEvent, Fetcher};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn first_paint_precedes_eof_and_image_decoding_is_deferred() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let tail = Arc::new(tokio::sync::Notify::new());
    let image = Arc::new(tokio::sync::Notify::new());
    let paths = Arc::new(Mutex::new(Vec::new()));
    let prefix = format!("<!doctype html><title>Progressive docs</title><style>main{{width:280px;padding:8px}}img{{max-width:100%}}</style><main><h1>First paint</h1><p>Visible before EOF.</p><img src='/diagram.svg' width=80 height=40 alt='loading diagram'><span style='display:none'><img src='/hidden.svg'></span>{}", " ".repeat(24 * 1024));
    let end = "<p id=tail>Final paragraph</p></main>";
    let server_tail = tail.clone();
    let server_image = image.clone();
    let seen = paths.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let tail = server_tail.clone();
            let image = server_image.clone();
            let prefix = prefix.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut data = [0; 4096];
                let size = stream.read(&mut data).await.unwrap();
                let request = String::from_utf8_lossy(&data[..size]);
                let path = request.split_ascii_whitespace().nth(1).unwrap_or("/");
                seen.lock().unwrap().push(path.to_owned());
                if path == "/" {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type:text/html\r\nContent-Length:{}\r\nConnection:close\r\n\r\n", prefix.len() + end.len());
                    stream.write_all(head.as_bytes()).await.unwrap();
                    stream.write_all(prefix.as_bytes()).await.unwrap();
                    tail.notified().await;
                    stream.write_all(end.as_bytes()).await.unwrap();
                } else {
                    image.notified().await;
                    let body = "<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40'><rect width='80' height='40' fill='#126abc'/></svg>";
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type:image/svg+xml\r\nContent-Length:{}\r\nConnection:close\r\n\r\n{body}", body.len());
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
        }
    });
    let fetcher = Fetcher::spawn(Default::default(), None, Arc::new(|| {}));
    fetcher.send(Command::Navigate {
        id: 7,
        url: format!("http://{address}/").parse().unwrap(),
        allow_downgrade: false,
        environment: Environment::default(),
    });
    let start = std::time::Instant::now();
    let mut preview = None;
    while preview.is_none() && start.elapsed().as_secs() < 5 {
        fetcher.drain_events(|event| match event {
            FetchEvent::Preview { page, styles, .. } => preview = Some((page, styles)),
            FetchEvent::Done { .. } => panic!("document finished before the held EOF"),
            _ => {}
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let (preview_page, preview_styles) = preview.expect("progressive preview before EOF");
    assert_eq!(
        preview_page.document.title().as_deref(),
        Some("Progressive docs")
    );
    assert_eq!(preview_page.image_cache.bytes(), 0);
    let mut fonts = cosmic_text::FontSystem::new();
    let frame = shell::viewport::headless_frame(
        &preview_page,
        &preview_styles,
        layout::Size::new(400.0, 300.0),
        &mut fonts,
    );
    assert!(frame.layout.stats.glyphs > 0);
    assert!(frame.pixels.iter().any(|&p| p != 0xffffff));
    println!(
        "progressive first paint: {:.3} ms while EOF is held",
        start.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(paths.lock().unwrap().as_slice(), &["/"]);
    tail.notify_one();
    let mut done = None;
    while done.is_none() && start.elapsed().as_secs() < 8 {
        fetcher.drain_events(|event| {
            if let FetchEvent::Done { result, .. } = event {
                done = Some(result.unwrap());
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let (page, styles) = done.expect("finished styled document");
    assert!(page.document.get_element_by_id("tail").is_some());
    assert_eq!(page.image_cache.bytes(), 0);
    image.notify_one();
    let mut decoded = false;
    while !decoded && start.elapsed().as_secs() < 10 {
        fetcher.drain_events(|event| {
            if let FetchEvent::Image { url, success, .. } = event {
                assert_eq!(url.path(), "/diagram.svg");
                assert!(success);
                decoded = true;
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(decoded);
    assert_eq!(page.image_cache.bytes(), 80 * 40 * 4);
    let ready = shell::viewport::headless_frame(
        &page,
        &styles,
        layout::Size::new(400.0, 300.0),
        &mut fonts,
    );
    assert!(ready
        .layout
        .boxes
        .iter()
        .flat_map(|b| &b.content)
        .any(|c| matches!(c, layout::Content::Image { .. })));
    assert!(!paths.lock().unwrap().iter().any(|s| s == "/hidden.svg"));
    fetcher.shutdown();
    server.abort();
}

#[tokio::test]
async fn stopped_stream_cannot_emit_a_finished_document() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type:text/html\r\nContent-Length:1000000\r\n\r\n",
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });
    let fetcher = Fetcher::spawn(Default::default(), None, Arc::new(|| {}));
    fetcher.send(Command::Navigate {
        id: 3,
        url: format!("http://{address}/").parse().unwrap(),
        allow_downgrade: false,
        environment: Environment::default(),
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    fetcher.send(Command::Stop { id: 3 });
    let mut stopped = false;
    for _ in 0..100 {
        fetcher.drain_events(|event| match event {
            FetchEvent::Stopped { id: 3 } => stopped = true,
            FetchEvent::Done { .. } => panic!("aborted stream emitted Done"),
            _ => {}
        });
        if stopped {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(stopped);
    fetcher.shutdown();
    server.abort();
}
