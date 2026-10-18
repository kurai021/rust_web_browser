//! Loopback-only synthetic fixture, not an authentication implementation.
use futures::{SinkExt, StreamExt};
use std::{io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
const INDEX: &str = include_str!("../../shell/tests/fixtures/web-demo/index.html");
const SCRIPT: &str = include_str!("../../shell/tests/fixtures/web-demo/demo.js");
const STYLE: &str = include_str!("../../shell/tests/fixtures/web-demo/style.css");
const WORKER: &str = include_str!("../../shell/tests/fixtures/web-demo/worker.js");
const BADGE: &str = include_str!("../../shell/tests/fixtures/web-demo/badge.svg");
const MODERN_INDEX: &str = include_str!("../../shell/tests/fixtures/modern-demo/index.html");
const MODERN_APP: &str = include_str!("../../shell/tests/fixtures/modern-demo/app.js");
const MODERN_API: &str = include_str!("../../shell/tests/fixtures/modern-demo/api.js");
const MODERN_STORE: &str = include_str!("../../shell/tests/fixtures/modern-demo/store.js");
const MODERN_VIEW: &str = include_str!("../../shell/tests/fixtures/modern-demo/view.js");
const MODERN_DETAILS: &str = include_str!("../../shell/tests/fixtures/modern-demo/details.js");
const MODERN_WORKER: &str = include_str!("../../shell/tests/fixtures/modern-demo/worker.js");
const MODERN_STYLE: &str = include_str!("../../shell/tests/fixtures/modern-demo/style.css");
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> io::Result<()> {
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "8097".into())
        .parse::<u16>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    println!("PHASE7_DEMO {origin}/");
    println!("PHASE8_DEMO {origin}/modern/");
    let board = Arc::new(std::sync::Mutex::new(vec![
        serde_json::json!({"id":1,"title":"Explore live module bindings","completed":false}),
        serde_json::json!({"id":2,"title":"Verify async fetch and microtasks","completed":false}),
        serde_json::json!({"id":3,"title":"Keep workers isolated","completed":true}),
    ]));
    let slots = Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let origin = origin.clone();
        let board = board.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) =
                tokio::time::timeout(Duration::from_secs(30), serve(stream, origin, board)).await
            {
                eprintln!("demo request deadline: {error}");
            }
        });
    }
}
async fn serve(
    mut stream: TcpStream,
    origin: String,
    board: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) -> io::Result<()> {
    let mut peek = [0; 1024];
    let n = stream.peek(&mut peek).await?;
    if String::from_utf8_lossy(&peek[..n]).starts_with("GET /ws ") {
        return websocket(stream, origin).await;
    }
    let mut data = Vec::new();
    let mut buffer = [0; 4096];
    let end = loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            return Ok(());
        }
        data.extend_from_slice(&buffer[..n]);
        if data.len() > 64 * 1024 {
            return Ok(());
        }
        if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let header = String::from_utf8_lossy(&data[..end]).into_owned();
    let path = header
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");
    let length = header
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return Ok(());
    }
    while data.len() < end + length {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            return Ok(());
        }
        data.extend_from_slice(&buffer[..n]);
    }
    if path == "/api/events" {
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\ndata: connected\n\n").await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        stream
            .write_all(b"event: update\nid: 2\ndata: ready\n\n")
            .await?;
        return Ok(());
    }
    let mut extra = String::new();
    let (mime,body)=match path{
        "/modern/app.js"=>("text/javascript",MODERN_APP.into()),"/modern/api.js"=>("text/javascript",MODERN_API.into()),"/modern/store.js"=>("text/javascript",MODERN_STORE.into()),"/modern/view.js"=>("text/javascript",MODERN_VIEW.into()),"/modern/details.js"=>("text/javascript",MODERN_DETAILS.into()),"/modern/worker.js"=>("text/javascript",MODERN_WORKER.into()),"/modern/style.css"=>("text/css",MODERN_STYLE.into()),
        path if path.starts_with("/modern")=>("text/html",MODERN_INDEX.into()),
        path if path=="/api/tasks"||path.starts_with("/api/tasks/")=>{
            let method=header.split_whitespace().next().unwrap_or("GET");let input=serde_json::from_slice::<serde_json::Value>(&data[end..end+length]).unwrap_or_default();let mut rows=board.lock().unwrap_or_else(|e|e.into_inner());
            let value=if method=="POST"&&rows.len()<1000{let id=rows.iter().filter_map(|row|row["id"].as_u64()).max().unwrap_or(0)+1;let title=input["title"].as_str().unwrap_or("Untitled").chars().take(120).collect::<String>();let item=serde_json::json!({"id":id,"title":title,"completed":false});rows.push(item.clone());serde_json::json!({"item":item})}else if method=="PATCH"{let id=path.rsplit('/').next().and_then(|id|id.parse::<u64>().ok()).unwrap_or(0);let item=rows.iter_mut().find(|row|row["id"].as_u64()==Some(id));if let Some(item)=item{item["completed"]=serde_json::Value::Bool(input["completed"].as_bool().unwrap_or(false));serde_json::json!({"item":item})}else{serde_json::json!({"error":"not found"})}}else{serde_json::json!({"items":*rows})};
            ("application/json",value.to_string())
        },
        "/demo.js"=>("text/javascript",SCRIPT.to_owned()),"/style.css"=>("text/css",STYLE.to_owned()),"/worker.js"=>("text/javascript",WORKER.to_owned()),"/badge.svg"=>("image/svg+xml",BADGE.to_owned()),
        "/api/data"=>("application/json",r#"{"rows":[{"label":"Alpha","value":48},{"label":"Beta","value":62},{"label":"Gamma","value":35},{"label":"Delta","value":55}]}"#.into()),
        "/api/login"=>{let value=serde_json::from_slice::<serde_json::Value>(&data[end..end+length]).unwrap_or_default();let ok=value["username"]=="demo"&&value["password"]=="rust";if ok{extra.push_str("Set-Cookie: session=demo-session; HttpOnly; SameSite=Lax; Max-Age=86400; Path=/\r\n");}("application/json",format!("{{\"loggedIn\":{ok}}}"))},
        "/api/session"=>{let ok=header.lines().any(|l|l.to_ascii_lowercase().starts_with("cookie:")&&l.contains("session=demo-session"));("application/json",format!("{{\"loggedIn\":{ok},\"user\":\"demo\"}}"))},
        _=>("text/html",INDEX.to_owned()),
    };
    let policy="default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'";
    let response=format!("HTTP/1.1 200 OK\r\nContent-Type: {mime};charset=UTF-8\r\nContent-Length: {}\r\nContent-Security-Policy: {policy}\r\nConnection: close\r\n{extra}\r\n{body}",body.len());
    stream.write_all(response.as_bytes()).await
}
#[allow(clippy::result_large_err)] // The handshake auxiliary defines its callback response type.
async fn websocket(stream: TcpStream, origin: String) -> io::Result<()> {
    use tokio_tungstenite::tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
        protocol::WebSocketConfig,
    };
    let mut socket = tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        move |request: &Request, response: Response| {
            if request
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok())
                != Some(origin.as_str())
            {
                let mut error = ErrorResponse::new(Some("Origin denied".into()));
                *error.status_mut() = StatusCode::FORBIDDEN;
                Err(error)
            } else {
                Ok(response)
            }
        },
        Some(
            WebSocketConfig::default()
                .max_message_size(Some(1024 * 1024))
                .max_frame_size(Some(1024 * 1024)),
        ),
    )
    .await
    .map_err(io::Error::other)?;
    while let Some(message) = socket.next().await {
        let message = message.map_err(io::Error::other)?;
        if message.is_close() {
            break;
        }
        if message.is_text() || message.is_binary() {
            socket.send(message).await.map_err(io::Error::other)?;
        }
    }
    Ok(())
}
