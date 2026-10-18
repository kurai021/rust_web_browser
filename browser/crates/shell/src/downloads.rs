//! Window-owned streaming downloads. Page content never supplies filesystem paths.
//! One actor owns the queue; at most five transfers hold sockets/files at once.
use futures::{FutureExt, StreamExt};
use profile::{Download, DownloadState as Status};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{mpsc, watch},
    task::JoinSet,
};
use url::Url;

const CONCURRENT: usize = 5;
#[derive(Clone, Debug)]
pub struct Source {
    pub force_prompt: bool,
    pub url: Url,
    pub top: Url,
    pub filename: String,
    pub bytes: Option<Arc<Vec<u8>>>,
}
#[derive(Clone, Debug)]
pub struct Update {
    pub download: Download,
    pub bytes_per_second: u64,
    pub eta: Option<u64>,
}
enum Command {
    Add(u64, Box<Source>, PathBuf),
    Pause(u64),
    Resume(u64),
    Cancel(u64),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Running,
    Pause,
    Cancel,
}
type Updates = Arc<Mutex<BTreeMap<u64, Update>>>;

pub struct Manager {
    commands: mpsc::Sender<Command>,
    updates: Updates,
    next: AtomicU64,
    worker: Option<std::thread::JoinHandle<()>>,
    closing: watch::Sender<bool>,
}
impl Manager {
    pub fn spawn(
        client: Arc<net::Client>,
        initial: Vec<Download>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel(64);
        let updates = Arc::new(Mutex::new(BTreeMap::new()));
        let out = updates.clone();
        let next = initial
            .iter()
            .map(|d| d.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let (closing, closed) = watch::channel(false);
        let worker = std::thread::Builder::new()
            .name("downloads".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(2)
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(run(client, initial, receiver, closed, out, wake))
                    }
                    Err(error) => eprintln!("browser: downloads runtime: {error}"),
                }
            })?;
        Ok(Self {
            commands,
            updates,
            next: AtomicU64::new(next),
            worker: Some(worker),
            closing,
        })
    }
    pub fn enqueue(&self, source: Source, destination: PathBuf) -> Result<u64, String> {
        if source.url.as_str().len() > 8192
            || source.top.as_str().len() > 8192
            || !matches!(source.url.scheme(), "http" | "https" | "blob")
        {
            return Err("Unsupported download URL".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.commands
            .try_send(Command::Add(id, Box::new(source), destination))
            .map_err(|e| e.to_string())?;
        Ok(id)
    }
    pub fn pause(&self, id: u64) -> Result<(), String> {
        self.commands
            .try_send(Command::Pause(id))
            .map_err(|e| e.to_string())
    }
    pub fn resume(&self, id: u64) -> Result<(), String> {
        self.commands
            .try_send(Command::Resume(id))
            .map_err(|e| e.to_string())
    }
    pub fn cancel(&self, id: u64) -> Result<(), String> {
        self.commands
            .try_send(Command::Cancel(id))
            .map_err(|e| e.to_string())
    }
    pub fn drain(&self) -> Vec<Update> {
        std::mem::take(&mut *self.updates.lock().unwrap_or_else(|e| e.into_inner()))
            .into_values()
            .collect()
    }
    pub fn shutdown(&mut self) {
        if self.worker.is_none() {
            return;
        }
        let _ = self.closing.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn publish(out: &Updates, wake: &Arc<dyn Fn() + Send + Sync>, row: &Download, rate: u64) {
    let eta = row
        .total
        .and_then(|total| total.saturating_sub(row.received).checked_div(rate));
    out.lock().unwrap_or_else(|e| e.into_inner()).insert(
        row.id,
        Update {
            download: row.clone(),
            bytes_per_second: rate,
            eta,
        },
    );
    wake();
}
async fn run(
    client: Arc<net::Client>,
    initial: Vec<Download>,
    mut commands: mpsc::Receiver<Command>,
    mut closing: watch::Receiver<bool>,
    out: Updates,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut rows: BTreeMap<u64, Download> = initial
        .into_iter()
        .take(profile::MAX_DOWNLOADS)
        .map(|mut row| {
            if matches!(row.state, Status::Running | Status::Queued) {
                row.state = Status::Paused;
            }
            (row.id, row)
        })
        .collect();
    let mut queue = VecDeque::new();
    let mut sources = BTreeMap::<u64, Source>::new();
    let mut controls = BTreeMap::<u64, watch::Sender<Control>>::new();
    let mut tasks = JoinSet::new();
    loop {
        while tasks.len() < CONCURRENT {
            let Some(id) = queue.pop_front() else {
                break;
            };
            let Some(row) = rows.get_mut(&id).filter(|r| r.state == Status::Queued) else {
                continue;
            };
            row.state = Status::Running;
            publish(&out, &wake, row, 0);
            let (control, receiver) = watch::channel(Control::Running);
            controls.insert(id, control);
            let row = row.clone();
            let backup = row.clone();
            let source = sources.get(&id).cloned();
            let client = client.clone();
            let out = out.clone();
            let wake = wake.clone();
            tasks.spawn(async move {
                match std::panic::AssertUnwindSafe(transfer(
                    client, row, source, receiver, out, wake,
                ))
                .catch_unwind()
                .await
                {
                    Ok(row) => row,
                    Err(_) => {
                        let mut row = backup;
                        row.state = Status::Failed;
                        row.message = "Transfer stopped unexpectedly; partial file retained".into();
                        row
                    }
                }
            });
        }
        tokio::select! {
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(row)) = result { controls.remove(&row.id); if row.state == Status::Complete || row.state == Status::Cancelled { sources.remove(&row.id); } publish(&out, &wake, &row, 0); rows.insert(row.id, row); }
            }
            command = async { tokio::select! { biased; _ = closing.changed() => None, command = commands.recv() => command } } => {
                match command {
                    Some(Command::Add(id, source, destination)) => {
                        if rows.len() >= profile::MAX_DOWNLOADS { if let Some(old) = rows.iter().find(|(_, d)| matches!(d.state, Status::Complete | Status::Cancelled)).map(|(id, _)| *id) { rows.remove(&old); } }
                        if rows.len() >= profile::MAX_DOWNLOADS { let row = Download { id, state: Status::Failed, message: "Download history/queue limit reached".into(), ..Default::default() }; publish(&out, &wake, &row, 0); continue; }
                        let mut row = Download { id, url: profile::clean_url(source.url.as_str()).map_or_else(|| source.url.to_string(), |u| u.to_string()), top: source.top.to_string(), destination, started: profile::now(), ..Default::default() };
                        if let Err(error) = reserve(&mut row).await { row.state = Status::Failed; row.message = error; }
                        else { queue.push_back(id); sources.insert(id, *source); }
                        publish(&out, &wake, &row, 0); rows.insert(id, row);
                    }
                    Some(Command::Pause(id)) => {
                        if let Some(control) = controls.get(&id) { let _ = control.send(Control::Pause); }
                        else if let Some(row) = rows.get_mut(&id).filter(|r| r.state == Status::Queued) { row.state = Status::Paused; publish(&out, &wake, row, 0); }
                    }
                    Some(Command::Resume(id)) => {
                        if let Some(row) = rows.get_mut(&id).filter(|r| matches!(r.state, Status::Paused | Status::Failed) && !r.partial.as_os_str().is_empty()) { row.state = Status::Queued; queue.push_back(id); publish(&out, &wake, row, 0); }
                    }
                    Some(Command::Cancel(id)) => {
                        if let Some(control) = controls.get(&id) { let _ = control.send(Control::Cancel); }
                        else if let Some(row) = rows.get_mut(&id).filter(|r| !matches!(r.state, Status::Complete | Status::Cancelled)) { cleanup(row).await; row.state = Status::Cancelled; row.message = "Cancelled; partial removed".into(); sources.remove(&id); publish(&out, &wake, row, 0); }
                    }
                    None => {
                        for control in controls.values() { let _ = control.send(Control::Pause); }
                        while let Some(result) = tasks.join_next().await { if let Ok(row) = result { publish(&out, &wake, &row, 0); rows.insert(row.id, row); } }
                        for row in rows.values_mut().filter(|r| r.state == Status::Queued) { row.state = Status::Paused; row.message = "Paused when browser closed".into(); publish(&out, &wake, row, 0); }
                        break;
                    }
                }
            }
        }
    }
}
async fn reserve(row: &mut Download) -> Result<(), String> {
    let parent = row
        .destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Choose a download directory")?
        .to_owned();
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|e| e.to_string())?;
    let original = row
        .destination
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid filename")?
        .to_owned();
    let name = sanitize_filename(&original);
    let stem = Path::new(&name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = Path::new(&name)
        .extension()
        .and_then(|s| s.to_str())
        .map_or_else(String::new, |s| format!(".{s}"));
    for number in 0..10_000 {
        let name = if number == 0 {
            name.clone()
        } else {
            format!("{stem} ({number}){ext}")
        };
        let target = parent.join(name);
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        match options.open(&target).await {
            Ok(file) => {
                drop(file);
                row.destination = target;
                row.partial = parent.join(format!(".browser-download-{}.part", row.id));
                if let Err(error) = options.open(&row.partial).await {
                    let _ = tokio::fs::remove_file(&row.destination).await;
                    row.partial = PathBuf::new();
                    return Err(error.to_string());
                }
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Too many duplicate filenames".into())
}
async fn cleanup(row: &Download) {
    if !valid_partial(row) {
        return;
    }
    let _ = tokio::fs::remove_file(&row.partial).await;
    if tokio::fs::symlink_metadata(&row.destination)
        .await
        .is_ok_and(|m| m.is_file() && m.len() == 0)
    {
        let _ = tokio::fs::remove_file(&row.destination).await;
    }
}
fn valid_partial(row: &Download) -> bool {
    row.partial
        .file_name()
        .is_some_and(|name| name == format!(".browser-download-{}.part", row.id).as_str())
        && row.partial.parent() == row.destination.parent()
}
async fn transfer(
    client: Arc<net::Client>,
    mut row: Download,
    source: Option<Source>,
    mut control: watch::Receiver<Control>,
    out: Updates,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Download {
    let started = Instant::now();
    let initial = row.received;
    for attempt in 0..=3 {
        let result = transfer_once(
            &client,
            &mut row,
            source.as_ref(),
            &mut control,
            &out,
            &wake,
            started,
            initial,
        )
        .await;
        match result {
            Ok(()) => {
                row.state = Status::Complete;
                if !row.message.starts_with("Executable content") {
                    row.message = if row.message.contains("restarted") {
                        "Saved; server restarted the transfer; never opened automatically"
                    } else {
                        "Saved; never opened automatically"
                    }
                    .into();
                }
                break;
            }
            Err(error) if *control.borrow() != Control::Running => {
                if *control.borrow() == Control::Cancel {
                    cleanup(&row).await;
                    row.state = Status::Cancelled;
                    row.message = "Cancelled; partial removed".into();
                } else {
                    row.state = Status::Paused;
                    row.message = "Paused; resume with Range when supported".into();
                }
                let _ = error;
                break;
            }
            Err(error)
                if attempt < 3 && !error.starts_with("Local:") && !error.starts_with("HTTP 4") =>
            {
                row.message = format!("Retry {}/3: {error}", attempt + 1);
                publish(&out, &wake, &row, 0);
                tokio::select! { _ = tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))) => {}, _ = control.changed() => {} }
            }
            Err(error) => {
                row.state = Status::Failed;
                row.message = error;
                break;
            }
        }
    }
    row
}
#[allow(clippy::too_many_arguments)]
async fn transfer_once(
    client: &net::Client,
    row: &mut Download,
    source: Option<&Source>,
    control: &mut watch::Receiver<Control>,
    out: &Updates,
    wake: &Arc<dyn Fn() + Send + Sync>,
    started: Instant,
    initial: u64,
) -> Result<(), String> {
    if !valid_partial(row) {
        return Err("Local: invalid partial file identity".into());
    }
    let metadata = tokio::fs::symlink_metadata(&row.partial)
        .await
        .map_err(|e| format!("Local: {e}"))?;
    if !metadata.is_file() || metadata.len() != row.received {
        return Err("Local: partial file was changed outside the browser".into());
    }
    if *control.borrow() != Control::Running {
        return Err("Paused".into());
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(target_os = "linux")]
    {
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let mut file = options
        .open(&row.partial)
        .await
        .map_err(|e| format!("Local: {e}"))?;
    let mut prefix = Vec::new();
    if let Some(bytes) = source.and_then(|s| s.bytes.as_ref()) {
        file.set_len(0).await.map_err(|e| format!("Local: {e}"))?;
        row.received = 0;
        row.total = Some(bytes.len() as u64);
        for chunk in bytes.chunks(64 * 1024) {
            if *control.borrow() != Control::Running {
                file.flush().await.map_err(|e| format!("Local: {e}"))?;
                return Err("Paused".into());
            }
            file.write_all(chunk)
                .await
                .map_err(|e| format!("Local: {e}"))?;
            row.received += chunk.len() as u64;
            if prefix.is_empty() {
                prefix.extend_from_slice(&chunk[..chunk.len().min(512)]);
            }
        }
    } else {
        let url = source
            .map(|s| s.url.clone())
            .or_else(|| Url::parse(&row.url).ok())
            .ok_or("Local: invalid download URL")?;
        let top = Url::parse(&row.top).unwrap_or_else(|_| url.clone());
        let validator = row
            .etag
            .as_deref()
            .filter(|s| !s.starts_with("W/"))
            .or(row.modified.as_deref());
        let range = row.received > 0 && validator.is_some();
        let mut headers = vec![("Accept-Encoding".into(), "identity".into())];
        if range {
            headers.push(("Range".into(), format!("bytes={}-", row.received)));
            headers.push(("If-Range".into(), validator.unwrap_or_default().into()));
        }
        let (head, mut stream) = tokio::select! {
            result = client.open_get(&url, &top, headers, Duration::from_secs(24 * 60 * 60)) => result.map_err(|e| e.to_string())?,
            _ = control.changed() => return Err("Paused".into()),
        };
        if range
            && head.status == 416
            && head
                .header("content-range")
                .and_then(|s| s.strip_prefix("bytes */"))
                .and_then(|s| s.parse::<u64>().ok())
                == Some(row.received)
        {
            let unchanged = row
                .etag
                .as_deref()
                .filter(|s| !s.starts_with("W/"))
                .is_some_and(|tag| head.header("etag") == Some(tag))
                || row
                    .modified
                    .as_deref()
                    .is_some_and(|date| head.header("last-modified") == Some(date));
            if unchanged {
                row.total = Some(row.received);
                drop(stream);
                return finish_file(file, row, prefix).await;
            }
        }
        if !(200..300).contains(&head.status) {
            return Err(format!("HTTP {}", head.status));
        }
        if head
            .header("content-encoding")
            .is_some_and(|s| !s.eq_ignore_ascii_case("identity"))
        {
            return Err(
                "Local: server ignored identity encoding; refusing unstable resume offsets".into(),
            );
        }
        if range && head.status == 206 {
            let (start, total) =
                parse_content_range(head.header("content-range").unwrap_or_default())
                    .ok_or("Local: invalid Content-Range")?;
            if start != row.received
                || row.total.is_some_and(|n| n != total)
                || row
                    .etag
                    .as_deref()
                    .is_some_and(|tag| head.header("etag").is_some_and(|new| new != tag))
            {
                return Err("Local: resume response does not match the partial file".into());
            }
            row.total = Some(total);
        } else {
            if head.status == 206 {
                return Err("Local: unexpected partial response".into());
            }
            if row.received > 0 {
                row.message =
                    "Server cannot resume this representation; restarted from zero".into();
            }
            file.set_len(0).await.map_err(|e| format!("Local: {e}"))?;
            row.received = 0;
            row.total = head.header("content-length").and_then(|s| s.parse().ok());
        }
        file.seek(std::io::SeekFrom::Start(row.received))
            .await
            .map_err(|e| format!("Local: {e}"))?;
        row.etag = head.header("etag").map(str::to_owned);
        row.modified = head.header("last-modified").map(str::to_owned);
        row.mime = head
            .header("content-type")
            .unwrap_or("application/octet-stream")
            .chars()
            .take(255)
            .collect();
        row.url = profile::clean_url(head.url.as_str())
            .map_or_else(|| head.url.to_string(), |u| u.to_string());
        let mut last_report = Instant::now();
        let result: Result<(), String> = async {
            loop {
                let next = tokio::select! { result = tokio::time::timeout(Duration::from_secs(30), stream.next()) => result.map_err(|_| "Body idle timeout".to_owned())?, _ = control.changed() => return Err("Paused".into()) };
                let Some(chunk) = next else { break; }; let chunk = chunk.map_err(|e| e.to_string())?;
                if row.total.is_some_and(|total| row.received.saturating_add(chunk.len() as u64) > total) { return Err("Local: response exceeds declared size".into()); }
                if row.received == 0 { prefix.extend_from_slice(&chunk[..chunk.len().min(512)]); }
                file.write_all(&chunk).await.map_err(|e| format!("Local: {e}"))?; row.received += chunk.len() as u64;
                if last_report.elapsed() >= Duration::from_millis(200) {
                    let rate = (row.received.saturating_sub(initial) as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
                    publish(out, wake, row, rate); last_report = Instant::now();
                }
            }
            if row.total.is_some_and(|n| n != row.received) { return Err("Incomplete response".into()); } Ok(())
        }.await;
        file.flush().await.map_err(|e| format!("Local: {e}"))?;
        result?;
    }
    finish_file(file, row, prefix).await
}
async fn finish_file(
    mut file: tokio::fs::File,
    row: &mut Download,
    mut prefix: Vec<u8>,
) -> Result<(), String> {
    file.flush().await.map_err(|e| format!("Local: {e}"))?;
    if prefix.is_empty() && row.received > 0 {
        file.seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(|e| format!("Local: {e}"))?;
        prefix.resize(512, 0);
        let n = file
            .read(&mut prefix)
            .await
            .map_err(|e| format!("Local: {e}"))?;
        prefix.truncate(n);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|e| format!("Local: {e}"))?;
    }
    file.sync_all().await.map_err(|e| format!("Local: {e}"))?;
    drop(file);
    if tokio::fs::symlink_metadata(&row.destination)
        .await
        .is_ok_and(|m| !m.is_file() || m.len() != 0)
    {
        return Err("Local: destination was changed outside the browser".into());
    }
    #[cfg(target_os = "linux")]
    {
        let _ = xattr::set(&row.partial, "user.xdg.origin.url", row.url.as_bytes());
        let _ = xattr::set(&row.partial, "user.browser.quarantined", b"1");
    }
    if executable(&row.destination.to_string_lossy(), &prefix) {
        row.message = "Executable content may harm your computer; saved without execute permission and not opened".into();
    }
    if tokio::fs::symlink_metadata(&row.destination).await.is_ok() {
        tokio::fs::remove_file(&row.destination)
            .await
            .map_err(|e| format!("Local: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        &row.partial,
        rustix::fs::CWD,
        &row.destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| format!("Local: {e}"))?;
    #[cfg(not(target_os = "linux"))]
    {
        tokio::fs::hard_link(&row.partial, &row.destination)
            .await
            .map_err(|e| format!("Local: {e}"))?;
        tokio::fs::remove_file(&row.partial)
            .await
            .map_err(|e| format!("Local: {e}"))?;
    }
    Ok(())
}
pub fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (
        start.parse::<u64>().ok()?,
        end.parse::<u64>().ok()?,
        total.parse::<u64>().ok()?,
    );
    (start <= end && end < total).then_some((start, total))
}
pub fn sanitize_filename(source: &str) -> String {
    let mut name: String = source
        .chars()
        .filter(|c| {
            !c.is_control()
                && !"/\\:<>\"|?*".contains(*c)
                && !(('\u{202a}'..='\u{202e}').contains(c))
                && !(('\u{2066}'..='\u{2069}').contains(c))
        })
        .take(180)
        .collect();
    name = name.trim_matches(['.', ' ']).to_owned();
    while name.len() > 220 {
        name.pop();
    }
    if name.is_empty() {
        "download".into()
    } else {
        name
    }
}
pub fn suggested_filename(url: &Url, disposition: Option<&str>) -> String {
    if let Some(header) = disposition {
        let mut fallback = None;
        for field in header.split(';').skip(1) {
            if let Some((key, value)) = field.trim().split_once('=') {
                let value = value.trim().trim_matches('"');
                if key.eq_ignore_ascii_case("filename*") {
                    if let Some(encoded) = value
                        .strip_prefix("UTF-8''")
                        .or_else(|| value.strip_prefix("utf-8''"))
                    {
                        return sanitize_filename(
                            &percent_encoding::percent_decode_str(encoded).decode_utf8_lossy(),
                        );
                    }
                }
                if key.eq_ignore_ascii_case("filename") {
                    fallback = Some(sanitize_filename(value));
                }
            }
        }
        if let Some(name) = fallback {
            return name;
        }
    }
    sanitize_filename(
        &percent_encoding::percent_decode_str(
            url.path_segments()
                .and_then(|mut s| s.next_back())
                .filter(|s| !s.is_empty())
                .unwrap_or("download"),
        )
        .decode_utf8_lossy(),
    )
}
pub fn executable(name: &str, prefix: &[u8]) -> bool {
    let name = name.to_ascii_lowercase();
    [
        ".sh",
        ".appimage",
        ".deb",
        ".exe",
        ".msi",
        ".desktop",
        ".run",
        ".rpm",
    ]
    .iter()
    .any(|s| name.ends_with(s))
        || prefix.starts_with(b"\x7fELF")
        || prefix.starts_with(b"MZ")
        || prefix.starts_with(b"#!")
}
