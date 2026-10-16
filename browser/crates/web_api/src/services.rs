//! Cancellable, bounded services. VM state never crosses this message boundary.
use crate::{
    http,
    policy::Csp,
    runtime::{Command, Completion},
    streams::SseParser,
    *,
};
use futures::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest,
    protocol::{frame::coding::CloseCode, CloseFrame, Message, WebSocketConfig},
};

mod worker;
enum SocketCommand {
    Send(Vec<u8>, bool),
    Close(u16, String),
}
struct SocketOptions {
    url: Url,
    protocols: Vec<String>,
    document: Url,
    policy: Csp,
}
pub struct Services {
    handle: tokio::runtime::Handle,
    client: Arc<net::Client>,
    tx: mpsc::Sender<Completion>,
    rx: mpsc::Receiver<Completion>,
    tasks: HashMap<u64, tokio::task::JoinHandle<()>>,
    sockets: HashMap<u64, mpsc::Sender<SocketCommand>>,
    workers: HashMap<u64, worker::Worker>,
}
impl Services {
    pub fn new(client: Arc<net::Client>, handle: tokio::runtime::Handle) -> Self {
        let (tx, rx) = mpsc::channel(128);
        Self {
            handle,
            client,
            tx,
            rx,
            tasks: HashMap::new(),
            sockets: HashMap::new(),
            workers: HashMap::new(),
        }
    }
    /// UI-facing commands are returned to the shell, never executed on an I/O task.
    pub fn dispatch(&mut self, command: Command) -> Result<Option<Command>> {
        self.tasks.retain(|_, task| !task.is_finished());
        match command {
            Command::Fetch {
                id,
                request,
                document,
                policy,
            } => {
                self.capacity()?;
                self.abort(id);
                let (client, tx) = (self.client.clone(), self.tx.clone());
                self.tasks.insert(
                    id,
                    self.handle.spawn(async move {
                        let result = http::fetch(&client, &document, &policy, request).await;
                        let _ = tx.send(Completion::Fetch { id, result }).await;
                    }),
                );
            }
            Command::Abort { id } => self.abort(id),
            Command::SocketOpen {
                id,
                url,
                protocols,
                document,
                policy,
            } => {
                self.capacity()?;
                let (tx, rx) = mpsc::channel(16);
                self.sockets.insert(id, tx);
                let (client, tx) = (self.client.clone(), self.tx.clone());
                self.tasks.insert(
                    id,
                    self.handle.spawn(async move {
                        if let Err(error) = socket(
                            &client,
                            id,
                            SocketOptions {
                                url,
                                protocols,
                                document,
                                policy,
                            },
                            rx,
                            &tx,
                        )
                        .await
                        {
                            let _ = tx.send(Completion::Error { id, error }).await;
                            let _ = tx
                                .send(Completion::Closed {
                                    id,
                                    code: 1006,
                                    reason: String::new(),
                                })
                                .await;
                        }
                    }),
                );
            }
            Command::SocketSend { id, data, text } => {
                self.sockets
                    .get(&id)
                    .ok_or_else(|| ApiError::new("InvalidStateError", "socket closed"))?
                    .try_send(SocketCommand::Send(data, text))
                    .map_err(|_| ApiError::new("QuotaExceededError", "socket queue"))?;
            }
            Command::SocketClose { id, code, reason } => {
                if let Some(socket) = self.sockets.get(&id) {
                    socket
                        .try_send(SocketCommand::Close(code, reason))
                        .map_err(|_| ApiError::new("QuotaExceededError", "socket queue"))?;
                }
            }
            Command::EventSource {
                id,
                url,
                credentials,
                document,
                policy,
            } => {
                self.capacity()?;
                let (client, tx) = (self.client.clone(), self.tx.clone());
                self.tasks.insert(
                    id,
                    self.handle.spawn(async move {
                        if let Err(error) =
                            event_source(&client, id, url, credentials, document, policy, &tx).await
                        {
                            let _ = tx.send(Completion::Error { id, error }).await;
                        }
                    }),
                );
            }
            Command::WorkerStart { id, url, policy } => {
                if self.workers.len() >= 4 {
                    return Err(ApiError::new("QuotaExceededError", "worker count"));
                }
                self.workers.insert(
                    id,
                    worker::Worker::spawn(
                        id,
                        url,
                        policy,
                        self.client.clone(),
                        self.handle.clone(),
                        self.tx.clone(),
                    )?,
                );
            }
            Command::WorkerMessage { id, data } => self
                .workers
                .get(&id)
                .ok_or_else(|| ApiError::new("InvalidStateError", "worker terminated"))?
                .send(data)?,
            Command::WorkerStop { id } => {
                self.workers.remove(&id);
            }
            ui => return Ok(Some(ui)),
        }
        Ok(None)
    }
    fn capacity(&self) -> Result<()> {
        if self.tasks.len() >= 64 {
            Err(ApiError::new("QuotaExceededError", "service task limit"))
        } else {
            Ok(())
        }
    }
    fn abort(&mut self, id: u64) {
        if let Some(task) = self.tasks.remove(&id) {
            task.abort();
        }
        self.sockets.remove(&id);
    }
    pub fn poll(&mut self) -> Option<Completion> {
        let result = self.rx.try_recv().ok()?;
        if let Completion::Closed { id, .. } = &result {
            self.sockets.remove(id);
        }
        if let Completion::WorkerClosed { id } = &result {
            self.workers.remove(id);
        }
        Some(result)
    }
    pub fn is_idle(&self) -> bool {
        self.tasks.values().all(|t| t.is_finished())
            && self.workers.is_empty()
            && self.rx.is_empty()
    }
    pub fn cancel(&mut self) {
        for (_, task) in self.tasks.drain() {
            task.abort();
        }
        self.sockets.clear();
        self.workers.clear();
        while self.rx.try_recv().is_ok() {}
    }
}
impl Drop for Services {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn network(error: impl std::fmt::Display) -> ApiError {
    ApiError::new("NetworkError", error.to_string())
}
async fn socket(
    client: &net::Client,
    id: u64,
    options: SocketOptions,
    mut commands: mpsc::Receiver<SocketCommand>,
    events: &mpsc::Sender<Completion>,
) -> Result<()> {
    let SocketOptions {
        mut url,
        protocols,
        document,
        policy,
    } = options;
    let mut http = url.clone();
    let _ = http.set_scheme(if url.scheme() == "wss" {
        "https"
    } else {
        "http"
    });
    http = client.resource_url(&http).map_err(network)?;
    let _ = url.set_scheme(if http.scheme() == "https" {
        "wss"
    } else {
        "ws"
    });
    if !policy.allows("connect-src", &document, &url, None) {
        return Err(ApiError::new("SecurityError", "WebSocket policy"));
    }
    let mut request = url.as_str().into_client_request().map_err(network)?;
    request
        .headers_mut()
        .insert("origin", origin(&document)?.parse().map_err(network)?);
    request.headers_mut().insert(
        "user-agent",
        net::client::USER_AGENT_VALUE.parse().map_err(network)?,
    );
    let cookie = client.cookies.header(&http, &document, false);
    if !cookie.is_empty() {
        request
            .headers_mut()
            .insert("cookie", cookie.parse().map_err(network)?);
    }
    if !protocols.is_empty() {
        request.headers_mut().insert(
            "sec-websocket-protocol",
            protocols.join(", ").parse().map_err(network)?,
        );
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let (mut socket, response) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(network)?
    .map_err(network)?;
    let protocol = response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if !protocol.is_empty() && !protocols.contains(&protocol) {
        return Err(ApiError::new("SecurityError", "unexpected socket protocol"));
    }
    for value in response.headers().get_all("set-cookie") {
        if let Ok(value) = value.to_str() {
            client.cookies.set(&http, &document, value, false);
        }
    }
    events
        .send(Completion::Opened { id, protocol })
        .await
        .map_err(network)?;
    loop {
        tokio::select! {
            command=commands.recv()=>match command {
                Some(SocketCommand::Send(data,text))=>{let bytes=data.len();let message=if text{Message::text(String::from_utf8(data).map_err(network)?)}else{Message::binary(data)};
                    tokio::time::timeout(Duration::from_secs(10),socket.send(message)).await.map_err(network)?.map_err(network)?;
                    events.send(Completion::Sent{id,bytes}).await.map_err(network)?;}
                Some(SocketCommand::Close(code,reason))=>{
                    tokio::time::timeout(Duration::from_secs(5),socket.close(Some(CloseFrame{code:CloseCode::from(code),reason:reason.clone().into()}))).await.map_err(network)?.map_err(network)?;
                    events.send(Completion::Closed{id,code,reason}).await.map_err(network)?;return Ok(());}
                None=>return Ok(()),
            },
            message=socket.next()=>match message {
                Some(Ok(Message::Text(text)))=>{events.send(Completion::Text{id,text:text.to_string()}).await.map_err(network)?;}
                Some(Ok(Message::Binary(bytes)))=>{events.send(Completion::Binary{id,bytes:bytes.to_vec()}).await.map_err(network)?;}
                Some(Ok(Message::Close(frame)))=>{let(code,reason)=frame.map_or((1005,String::new()),|f|(u16::from(f.code),f.reason.to_string()));events.send(Completion::Closed{id,code,reason}).await.map_err(network)?;return Ok(());}
                Some(Ok(_))=>{},
                Some(Err(error))=>return Err(network(error)),
                None=>{events.send(Completion::Closed{id,code:1006,reason:String::new()}).await.map_err(network)?;return Ok(());}
            }
        }
    }
}
async fn event_source(
    client: &net::Client,
    id: u64,
    original: Url,
    credentials: bool,
    document: Url,
    policy: Csp,
    events: &mpsc::Sender<Completion>,
) -> Result<()> {
    let source = origin(&document)?;
    let mut last_id = String::new();
    loop {
        let mut url = original.clone();
        let mut stream = None;
        for hop in 0..=10 {
            url = client.resource_url(&url).map_err(network)?;
            if !policy.allows("connect-src", &document, &url, None) {
                return Err(ApiError::new("SecurityError", "EventSource policy"));
            }
            let cross = !same_origin(&document, &url);
            let mut headers = vec![("accept".into(), "text/event-stream".into())];
            if cross {
                headers.push(("origin".into(), source.clone()));
            }
            if !last_id.is_empty() {
                headers.push(("last-event-id".into(), last_id.clone()));
            }
            let (head, body) = client
                .exchange_stream(net::HttpRequest {
                    url: url.clone(),
                    method: "GET".into(),
                    headers,
                    body: Vec::new(),
                    top: document.clone(),
                    credentials: credentials || !cross,
                    max_bytes: 1024 * 1024,
                    timeout: Duration::from_secs(60),
                })
                .await
                .map_err(network)?;
            if cross {
                http::check_allow(&head, &source, credentials)?;
            }
            if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
                if hop == 10 {
                    return Err(ApiError::new("NetworkError", "EventSource redirects"));
                }
                url = web_url(&url, head.header("location").unwrap_or(""))?;
                continue;
            }
            if head.status == 204 {
                events
                    .send(Completion::Closed {
                        id,
                        code: 0,
                        reason: String::new(),
                    })
                    .await
                    .map_err(network)?;
                return Ok(());
            }
            if head.status != 200
                || !head
                    .header("content-type")
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case("text/event-stream"))
            {
                return Err(ApiError::new("NetworkError", "EventSource status/MIME"));
            }
            stream = Some(body);
            break;
        }
        let mut body =
            stream.ok_or_else(|| ApiError::new("NetworkError", "EventSource redirect"))?;
        events
            .send(Completion::Opened {
                id,
                protocol: String::new(),
            })
            .await
            .map_err(network)?;
        let mut parser = SseParser::with_id(last_id.clone());
        while let Some(chunk) = body.next().await {
            let Ok(chunk) = chunk else {
                break;
            };
            for event in parser.push(&chunk)? {
                events
                    .send(Completion::ServerEvent {
                        id,
                        event: event.event,
                        data: event.data,
                        last_id: event.id,
                    })
                    .await
                    .map_err(network)?;
            }
            last_id = parser.last_id().to_owned();
        }
        events
            .send(Completion::StreamRetry { id })
            .await
            .map_err(network)?;
        tokio::time::sleep(Duration::from_millis(parser.retry_ms())).await;
    }
}
