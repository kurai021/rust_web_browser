//! Offline end-to-end CSS sources, imports and viewport recomputation.
use css::Environment;
use net::{Client, FetchOptions};
use shell::page::load_page;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn external_embedded_inline_order_import_cycle_and_resize() {
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            let path = request.split_ascii_whitespace().nth(1).unwrap_or("/");
            seen.lock().unwrap().push(path.to_owned());
            let (mime,body)=match path {
                "/"=>("text/html","<!DOCTYPE html><link rel=stylesheet href='/css/main.css'><link rel=stylesheet href='/bad.css'><style>#target{font-size:30px;color:purple} @media(max-width:450px){#target{font-size:14px;background-color:#ffeecc}}</style><div><p id=target style='color:blue;margin-left:7px'>Text <b>bold</b></p></div>"),
                "/css/main.css"=>("text/css","@import '../shared.css'; @font-face{font-family:'Unused';src:url('/never.woff2')} #target{color:red!important;margin:2px 4px;font-family:sans-serif}"),
                "/shared.css"=>("text/css","@import '/css/main.css';p{font-size:20px} p b{color:inherit}"),
                "/bad.css"=>("text/html","#target{color:green!important}"),
                _=>("text/css",""),
            };
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let client = Client::new(FetchOptions::default()).unwrap();
    let url = format!("http://{address}/").parse().unwrap();
    let fetched = client.fetch(&url, &mut |_| {}).await.unwrap();
    let page = load_page(&client, fetched).await;
    let target = page.document.get_element_by_id("target").unwrap();
    let wide = page.computed_styles(Environment::default());
    let style = wide.get(target).unwrap();
    assert_eq!(style.get_property_value("color"), "rgb(255, 0, 0)");
    assert_eq!(style.font_size, 30.0);
    assert_eq!(style.get_property_value("margin-left"), "7px");
    assert_eq!(style.get_property_value("margin-right"), "4px");
    let narrow = page.computed_styles(Environment {
        width: 400.0,
        ..Environment::default()
    });
    assert_eq!(narrow.get(target).unwrap().font_size, 14.0);
    assert_eq!(
        narrow.get(target).unwrap().background_color,
        css::values::Color::rgb(255, 238, 204)
    );
    assert!(page.warnings.iter().any(|s| s.contains("cycle")));
    assert!(page.warnings.iter().any(|s| s.contains("MIME")));
    assert!(
        shell::fonts::load_fonts(&client, &page, Environment::default())
            .await
            .is_empty()
    );
    let paths = requests.lock().unwrap();
    assert_eq!(paths.iter().filter(|p| *p == "/css/main.css").count(), 1);
    assert!(!paths.iter().any(|p| p == "/never.woff2"));
    server.abort();
}

#[tokio::test]
async fn import_depth_is_bounded_before_opening_another_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            let path = request.split_ascii_whitespace().nth(1).unwrap();
            seen.lock().unwrap().push(path.to_owned());
            let index = path
                .trim_matches('/')
                .trim_end_matches(".css")
                .parse::<usize>()
                .unwrap();
            let body = format!(
                "@import '{}.css'; p{{font-size:{}px}}",
                index + 1,
                20 + index
            );
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection:close\r\n\r\n{body}",body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let client = Client::new(FetchOptions::default()).unwrap();
    let fetched = net::Fetched {
        url: format!("http://{address}/").parse().unwrap(),
        status: 200,
        content_type: Some("text/html".into()),
        bytes: b"<link rel=stylesheet href='0.css'><p>x</p>".to_vec(),
    };
    let page = load_page(&client, fetched).await;
    assert!(page.warnings.iter().any(|s| s.contains("depth")));
    assert_eq!(
        requests.lock().unwrap().len(),
        shell::page::MAX_IMPORT_DEPTH + 1
    );
    server.abort();
}
