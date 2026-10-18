//! Deterministic loopback fixtures for browser/session/download acceptance.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub const LARGE: u64 = 100 * 1024 * 1024;
pub struct Directory(pub std::path::PathBuf);
impl Directory {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "browser-phase9-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub struct Server {
    pub base: url::Url,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub active: Arc<AtomicUsize>,
    pub peak: Arc<AtomicUsize>,
    worker: tokio::task::JoinHandle<()>,
}
struct Connection(Arc<AtomicUsize>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
impl Server {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (seen, count, high) = (requests.clone(), active.clone(), peak.clone());
        let pattern = Arc::new((0..65536).map(|n| (n % 251) as u8).collect::<Vec<_>>());
        let worker = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{let Ok((stream,_))=result else{break;};let(seen,count,high,pattern)=(seen.clone(),count.clone(),high.clone(),pattern.clone());tasks.spawn(async move{let _=serve(stream,seen,count,high,pattern).await;});},
                    _=tasks.join_next(),if !tasks.is_empty()=>{}
                }
            }
        });
        Self {
            base,
            requests,
            active,
            peak,
            worker,
        }
    }
    pub fn url(&self, path: &str) -> url::Url {
        self.base.join(path).unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
async fn serve(
    mut stream: tokio::net::TcpStream,
    seen: Arc<Mutex<Vec<String>>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    pattern: Arc<Vec<u8>>,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buffer).await?;
        if n == 0 || request.len() + n > 32768 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..n]);
    }
    let request = String::from_utf8_lossy(&request).into_owned();
    seen.lock().unwrap().push(request.clone());
    let path = request.split_ascii_whitespace().nth(1).unwrap_or("/");
    if path == "/progressive" {
        let prefix=format!("<!doctype html><title>Provisional document</title><main><h1>Provisional</h1><p>Stopped safely.</p>{}"," ".repeat(24*1024));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type:text/html\r\nContent-Length:1000000\r\nConnection:close\r\n\r\n").await?;
        stream.write_all(prefix.as_bytes()).await?;
        let mut byte = [0u8; 1];
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(10), stream.read(&mut byte)).await;
        return Ok(());
    }
    if path.ends_with(".bin") {
        let count = active.fetch_add(1, Ordering::Relaxed) + 1;
        peak.fetch_max(count, Ordering::Relaxed);
        let _guard = Connection(active);
        let total = if path == "/large.bin" {
            LARGE
        } else {
            8 * 1024 * 1024
        };
        let range = request
            .lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("range"))
                    .map(|(_, v)| v.trim())
            })
            .and_then(|s| s.strip_prefix("bytes="))
            .and_then(|s| s.strip_suffix('-'))
            .and_then(|s| s.parse::<u64>().ok());
        let mut start = range.unwrap_or(0);
        let partial = range.is_some() && path != "/restart.bin";
        if !partial {
            start = 0;
        }
        if start >= total {
            stream.write_all(format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length:0\r\nContent-Range:bytes */{total}\r\nETag:\"fixture-v1\"\r\nConnection:close\r\n\r\n").as_bytes()).await?;
            return Ok(());
        }
        let content_range = if partial {
            format!(
                "Content-Range:bytes {}-{}/{total}\r\n",
                if path == "/wrong.bin" {
                    start + 1
                } else {
                    start
                },
                total - 1
            )
        } else {
            String::new()
        };
        stream.write_all(format!("HTTP/1.1 {}\r\nContent-Type:application/octet-stream\r\nContent-Disposition:attachment; filename=\"fixture.bin\"\r\nContent-Length:{}\r\nAccept-Ranges:bytes\r\nETag:\"fixture-v1\"\r\n{content_range}Connection:close\r\n\r\n",if partial{"206 Partial Content"}else{"200 OK"},total-start).as_bytes()).await?;
        if path == "/held.bin" {
            let mut byte = [0u8; 1];
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(10), stream.read(&mut byte))
                    .await;
            return Ok(());
        }
        let delay = if path == "/large.bin" { 1 } else { 4 };
        while start < total {
            let offset = (start % 65536) as usize;
            let n = ((total - start) as usize).min(pattern.len() - offset);
            stream.write_all(&pattern[offset..offset + n]).await?;
            start += n as u64;
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
        return Ok(());
    }
    let (mime, body) = if path == "/icon.svg" {
        ("image/svg+xml","<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><rect width='16' height='16' fill='#128878'/></svg>".into())
    } else {
        let id = path
            .rsplit('/')
            .next()
            .unwrap_or("0")
            .parse::<u32>()
            .unwrap_or(0);
        ("text/html",format!("<!doctype html><title>Tab {id}</title><link rel=icon href=/icon.svg><style>body{{font:16px sans-serif}}p{{margin:12px}}.hidden{{display:none}}</style><h1>Tab {id}</h1><p>café and café</p><span class=hidden>not painted</span><button id=inc>Increment</button><output id=count>{id}:0</output><a id=next href='/tab/9'>Next page</a><script>var owned={id};var count=0;sessionStorage.setItem('tab',String(owned));document.getElementById('inc').onclick=()=>document.getElementById('count').textContent=owned+':'+(++count)+':'+sessionStorage.getItem('tab');</script>"))
    };
    let body = if mime == "text/html" {
        body.replace(
            "<a id=next",
            "<a id=route href='#one'>Local route</a><a id=next",
        )
    } else {
        body
    };
    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type:{mime}\r\nContent-Length:{}\r\nConnection:close\r\n\r\n{body}",body.len()).as_bytes()).await?;
    Ok(())
}
