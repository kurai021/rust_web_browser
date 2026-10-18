//! Background fetcher (plan/04 §4.3.2, plan/03 §3.3).
//!
//! Network I/O never runs on the UI thread: a dedicated worker thread owns
//! a `tokio` runtime and an `net::Client`. The UI sends [`Command`]s and
//! drains [`FetchEvent`]s (polled in `about_to_wait`). A new navigation or
//! an explicit stop aborts the in-flight task — dropping the future cancels
//! the request (plan/04 cancellation rule).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::fonts::{load_fonts, FontAsset};
use crate::images::load_images;
use crate::page::Page;
use css::Environment;
use futures::{FutureExt, StreamExt};
use net::{Client, Error, FetchOptions, Progress, UserInput};
use tokio::task::JoinHandle as TokioHandle;
use url::Url;

/// Window-wide transport and origin storage, with distinct sessionStorage per tab.
/// Opening performs local startup I/O only; it never resolves or connects a host.
pub struct Shared {
    pub client: Arc<Client>,
    pub profile: Arc<web_api::storage::Profile>,
    directory: Option<PathBuf>,
    persistence: std::sync::Mutex<()>,
    next_session: AtomicU64,
}
impl Shared {
    pub fn open(options: FetchOptions, directory: Option<PathBuf>) -> Result<Arc<Self>, String> {
        Self::configured(options, directory, false)
    }
    pub fn configured(
        options: FetchOptions,
        directory: Option<PathBuf>,
        doh: bool,
    ) -> Result<Arc<Self>, String> {
        let client = Arc::new(Client::new_with_doh(options, doh).map_err(|e| e.to_string())?);
        let profile = Arc::new(
            web_api::storage::Profile::open(directory.clone()).map_err(|e| e.to_string())?,
        );
        if let Some(dir) = &directory {
            client
                .cookies
                .load(&dir.join("cookies.json"))
                .map_err(|e| e.to_string())?;
            *client.hsts() = net::HstsStore::load_from(&dir.join("hsts.json"));
        }
        Ok(Arc::new(Self {
            client,
            profile,
            directory,
            persistence: std::sync::Mutex::new(()),
            next_session: AtomicU64::new(1),
        }))
    }
    fn persist(&self) {
        let _guard = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
        persist_hsts(&self.client, &self.directory);
    }
}

/// Commands from the UI thread to the worker.
#[derive(Debug)]
pub enum Command {
    StopNavigation {
        id: u64,
    },
    NavigationPolicy {
        javascript: bool,
        sites: std::collections::BTreeMap<String, bool>,
        cache: net::navigation::CacheMode,
    },
    WebDecision {
        id: u64,
        request: u64,
        name: String,
        decision: String,
        action: crate::web_ui::Action,
    },
    RevokePermissions {
        id: u64,
        all: bool,
    },
    OpenPopup {
        url: Url,
    },
    WebCompletion {
        id: u64,
        completion: web_api::WebCompletion,
    },
    DomEvent {
        id: u64,
        event: crate::script_worker::DomCommand,
    },
    /// Load `url`. Aborts anything in flight. `allow_downgrade` enables the
    /// https→http fallback for bare-host input (plan/04 §4.3.3).
    Navigate {
        id: u64,
        url: Url,
        allow_downgrade: bool,
        environment: Environment,
    },
    /// Recompute media-dependent styles off the UI thread after a resize.
    Restyle {
        id: u64,
        revision: u64,
        page: Arc<Page>,
        environment: Environment,
    },
    /// Abort the in-flight fetch, if any.
    Stop {
        id: u64,
    },
    /// Persist HSTS under `dir` and exit the worker.
    Shutdown,
}

/// Events from the worker to the UI thread.
#[derive(Debug)]
pub enum FetchEvent {
    Committed {
        id: u64,
        url: Url,
    },
    Favicon {
        id: u64,
        image: Arc<layout::RasterImage>,
    },
    Download {
        id: u64,
        source: crate::downloads::Source,
    },
    Crashed {
        id: u64,
        message: String,
    },
    WebRequest {
        id: u64,
        origin: String,
        command: web_api::WebCommand,
    },
    Dialog {
        id: u64,
        request: web_api::runtime::Dialog,
        reply: Sender<Option<String>>,
    },
    SameDocument {
        id: u64,
        url: Url,
        back: bool,
        forward: bool,
        replace: bool,
        traversal: Option<i32>,
    },
    WebNavigate {
        id: u64,
        url: Url,
        replace: bool,
    },
    ScriptPage {
        _lease: Arc<crate::script_worker::PageLease>,
        id: u64,
        page: Arc<Page>,
        styles: css::ComputedStyles,
        focused: Option<html::NodeId>,
        console: Vec<String>,
    },
    ScriptError {
        id: u64,
        message: String,
        stopped: bool,
    },
    Navigate {
        id: u64,
        url: Url,
    },
    /// A fetch started (progress bar on).
    Started {
        id: u64,
        url: Url,
    },
    /// Body progress update.
    Progress {
        id: u64,
        downloaded: usize,
        total: Option<u64>,
    },
    /// Consistent partial DOM/CSS, emitted while the HTML stream is open.
    Preview {
        id: u64,
        page: Arc<Page>,
        styles: css::ComputedStyles,
    },
    /// Fetch finished (success or typed failure).
    Done {
        id: u64,
        url: Url,
        result: Result<(Arc<Page>, css::ComputedStyles), Error>,
    },
    /// Font-display swap: fonts arrive after the styled fallback document.
    Fonts {
        id: u64,
        page: Arc<Page>,
        assets: Vec<FontAsset>,
    },
    Image {
        id: u64,
        page: Arc<Page>,
        url: Url,
        success: bool,
    },
    Styled {
        id: u64,
        revision: u64,
        page: Arc<Page>,
        styles: css::ComputedStyles,
    },
    /// Nothing in flight anymore after a stop.
    Stopped {
        id: u64,
    },
}

/// Handle held by the UI thread.
pub struct Fetcher {
    commands: Sender<Command>,
    events: Receiver<FetchEvent>,
    worker: Option<JoinHandle<()>>,
    active_script: Arc<AtomicU64>,
    shared: Arc<Shared>,
}

impl Fetcher {
    /// Spawn the worker. `profile_dir` enables HSTS persistence; `None`
    /// keeps HSTS in memory only (fail-safe default). `wake` is called after
    /// every queued event so the UI thread can leave `ControlFlow::Wait`.
    #[must_use]
    pub fn spawn(
        options: FetchOptions,
        profile_dir: Option<PathBuf>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self::spawn_shared(
            Shared::open(options, profile_dir).expect("test/startup profile"),
            wake,
        )
        .expect("net worker thread spawns")
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    pub fn spawn_shared(
        shared: Arc<Shared>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let (commands_tx, commands_rx) = channel::<Command>();
        let (events_tx, events_rx) = channel::<FetchEvent>();
        let active_script = Arc::new(AtomicU64::new(0));
        let token = active_script.clone();
        let common = shared.clone();
        let crash_events = events_tx.clone();
        let crash_wake = wake.clone();
        let crash_token = token.clone();
        let worker = std::thread::Builder::new()
            .name("net-worker".to_owned())
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_main(commands_rx, events_tx, wake, common, token)
                }))
                .is_err()
                {
                    let _ = crash_events.send(FetchEvent::Crashed {
                        id: crash_token.fetch_add(1, Ordering::Relaxed),
                        message: "Tab network worker stopped unexpectedly; reload this tab".into(),
                    });
                    crash_wake();
                }
            })?;
        Ok(Self {
            commands: commands_tx,
            events: events_rx,
            worker: Some(worker),
            active_script,
            shared,
        })
    }

    /// Non-blocking drain of pending events.
    pub fn drain_events(&self, mut on_event: impl FnMut(FetchEvent)) {
        while let Ok(event) = self.events.try_recv() {
            on_event(event);
        }
    }

    /// Send a command (no-op when the worker is gone).
    pub fn send(&self, command: Command) {
        match &command {
            // A navigation commits its realm only after a renderable response.
            // Attachment responses preserve the old page and its event loop.
            Command::Navigate { .. } => {}
            Command::Stop { .. } | Command::Shutdown => {
                self.active_script.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        let _ = self.commands.send(command);
    }

    /// Stop the worker thread (also aborts any in-flight fetch).
    pub fn shutdown(mut self) {
        self.active_script.fetch_add(1, Ordering::Relaxed);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Fetcher {
    fn drop(&mut self) {
        self.active_script.fetch_add(1, Ordering::Relaxed);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn hsts_path(profile_dir: &Option<PathBuf>) -> Option<PathBuf> {
    profile_dir.as_ref().map(|dir| dir.join("hsts.json"))
}

fn worker_main(
    commands: Receiver<Command>,
    events: Sender<FetchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    shared: Arc<Shared>,
    active_script: Arc<AtomicU64>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .thread_name("net-io")
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    let client = shared.client.clone();
    let config = web_api::RuntimeConfig {
        profile: shared.profile.clone(),
        cookies: client.cookies.clone(),
        session: format!(
            "tab-{}-{}",
            std::process::id(),
            shared.next_session.fetch_add(1, Ordering::Relaxed)
        ),
        ..Default::default()
    };
    let scripts = match crate::script_worker::ScriptWorker::spawn_configured(
        client.clone(),
        events.clone(),
        wake.clone(),
        active_script.clone(),
        config,
    ) {
        Ok(worker) => Arc::new(std::sync::Mutex::new(worker)),
        Err(error) => {
            eprintln!("browser: script worker failed: {error}");
            return;
        }
    };
    let mut in_flight: Option<TokioHandle<()>> = None;
    let mut restyling: Option<TokioHandle<()>> = None;
    let mut javascript = true;
    let mut site_javascript = std::collections::BTreeMap::new();
    let mut cache = net::navigation::CacheMode::Normal;

    let abort_in_flight = |slot: &mut Option<TokioHandle<()>>| {
        if let Some(handle) = slot.take() {
            handle.abort();
        }
    };

    while let Ok(command) = commands.recv() {
        match command {
            Command::NavigationPolicy {
                javascript: enabled,
                sites,
                cache: mode,
            } => {
                javascript = enabled;
                site_javascript = sites;
                cache = mode;
            }
            Command::WebDecision {
                id,
                request,
                name,
                decision,
                action,
            } => scripts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .decision(id, request, name, decision, action),
            Command::RevokePermissions { id, all } => scripts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .revoke(id, all),
            Command::OpenPopup { url } => {
                if let Ok(exe) = std::env::current_exe() {
                    if let Err(error) = std::process::Command::new(exe)
                        .arg(url.as_str())
                        .stdin(std::process::Stdio::null())
                        .spawn()
                    {
                        eprintln!("browser: popup window failed: {error}");
                    }
                }
            }
            Command::WebCompletion { id, completion } => scripts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .complete(id, completion),
            Command::DomEvent { id, event } => scripts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .event(id, event),
            Command::Navigate {
                id,
                url,
                allow_downgrade,
                environment,
            } => {
                abort_in_flight(&mut in_flight);
                abort_in_flight(&mut restyling);
                let client = Arc::clone(&client);
                let events = events.clone();
                let wake = Arc::clone(&wake);
                let shared = shared.clone();
                let scripts = scripts.clone();
                let active_script = active_script.clone();
                let site_javascript = site_javascript.clone();
                let crash_events = events.clone();
                let crash_wake = wake.clone();
                let crash_token = active_script.clone();
                let job = async move {
                    let _ = events.send(FetchEvent::Started {
                        id,
                        url: url.clone(),
                    });
                    wake();
                    let events_for_progress = events.clone();
                    let mut last_report = 0usize;
                    let mut parser = None::<html::Parser>;
                    let mut stream_url = None;
                    let mut previews = 0usize;
                    let mut next_preview = 16 * 1024;
                    let mut last_preview = None::<std::time::Instant>;
                    let mut committed = false;
                    let result = fetch_navigation(
                        &client,
                        &url,
                        allow_downgrade,
                        cache,
                        &mut |head, chunk, progress: Progress| {
                            if !committed {
                                committed = true;
                                active_script.store(id, Ordering::Relaxed);
                                let _ = events_for_progress.send(FetchEvent::Committed {
                                    id,
                                    url: head.url.clone(),
                                });
                                wake();
                            }
                            if stream_url.as_ref() != Some(&head.url) {
                                parser = Some(html::Parser::new(
                                    head.url.clone(),
                                    Box::new(html::NullSink),
                                    Default::default(),
                                ));
                                stream_url = Some(head.url.clone());
                            }
                            if let Some(parser) = &mut parser {
                                parser.push(chunk);
                            }
                            if previews < 8
                                && progress.downloaded >= next_preview
                                && last_preview.is_none_or(|t| {
                                    t.elapsed() >= std::time::Duration::from_millis(200)
                                })
                            {
                                next_preview = progress.downloaded.saturating_add(128 * 1024);
                                if let Some(document) =
                                    parser.as_ref().and_then(html::Parser::snapshot)
                                {
                                    if !document.get_elements_by_tag_name("body").is_empty() {
                                        let page = Arc::new(Page::preview(head, document));
                                        let styles = page.computed_styles(environment);
                                        let _ = events_for_progress.send(FetchEvent::Preview {
                                            id,
                                            page,
                                            styles,
                                        });
                                        previews += 1;
                                        last_preview = Some(std::time::Instant::now());
                                        wake();
                                    }
                                }
                            }
                            // Thin out progress events: every 32 KiB.
                            if progress.downloaded.saturating_sub(last_report) >= 32 * 1024 {
                                last_report = progress.downloaded;
                                let _ = events_for_progress.send(FetchEvent::Progress {
                                    id,
                                    downloaded: progress.downloaded,
                                    total: progress.total,
                                });
                                wake();
                            }
                        },
                    )
                    .await;
                    shared.persist();
                    match result {
                        Ok(Navigation::Document(mut fetched)) => {
                            if !site_javascript
                                .get(&fetched.url.origin().ascii_serialization())
                                .copied()
                                .unwrap_or(javascript)
                            {
                                fetched.script_policies.push("script-src 'none'".into());
                            }
                            active_script.store(id, Ordering::Relaxed);
                            scripts
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .initialize(id, fetched, environment);
                        }
                        Ok(Navigation::Download(source)) => {
                            let _ = events.send(FetchEvent::Download { id, source });
                            wake();
                        }
                        Err(error) => {
                            active_script.fetch_add(1, Ordering::Relaxed);
                            let _ = events.send(FetchEvent::Done {
                                id,
                                url,
                                result: Err(error),
                            });
                            wake();
                        }
                    }
                };
                in_flight=Some(runtime.spawn(async move{if std::panic::AssertUnwindSafe(job).catch_unwind().await.is_err(){crash_token.fetch_add(1,Ordering::Relaxed);let _=crash_events.send(FetchEvent::Crashed{id,message:"This tab's document loader stopped unexpectedly. Reload to recover.".into()});crash_wake();}}));
            }
            Command::Restyle {
                id,
                revision,
                page,
                environment,
            } => {
                abort_in_flight(&mut restyling);
                let events = events.clone();
                let wake = wake.clone();
                let client = client.clone();
                let crash_events = events.clone();
                let crash_wake = wake.clone();
                let job = async move {
                    let styles = page.computed_styles(environment);
                    let _ = events.send(FetchEvent::Styled {
                        id,
                        revision,
                        page: page.clone(),
                        styles: styles.clone(),
                    });
                    wake();
                    let assets = load_fonts(&client, &page, environment).await;
                    if !assets.is_empty() {
                        let _ = events.send(FetchEvent::Fonts {
                            id,
                            page: page.clone(),
                            assets,
                        });
                        wake();
                    }
                    load_images(&client, &page, &styles, |url, success| {
                        let _ = events.send(FetchEvent::Image {
                            id,
                            page: page.clone(),
                            url,
                            success,
                        });
                        wake();
                    })
                    .await;
                };
                restyling = Some(runtime.spawn(async move {
                    if std::panic::AssertUnwindSafe(job)
                        .catch_unwind()
                        .await
                        .is_err()
                    {
                        let _ = crash_events.send(FetchEvent::Crashed {
                            id,
                            message:
                                "This tab's style loader stopped unexpectedly. Reload to recover."
                                    .into(),
                        });
                        crash_wake();
                    }
                }));
            }
            Command::StopNavigation { id } | Command::Stop { id } => {
                abort_in_flight(&mut in_flight);
                abort_in_flight(&mut restyling);
                let _ = events.send(FetchEvent::Stopped { id });
                wake();
            }
            Command::Shutdown => {
                abort_in_flight(&mut in_flight);
                abort_in_flight(&mut restyling);
                scripts.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
                shared.persist();
                break;
            }
        }
    }
    runtime.shutdown_background();
}

/// Best-effort HSTS persistence; failures never break navigation.
fn persist_hsts(client: &Client, profile_dir: &Option<PathBuf>) {
    if let Some(path) = hsts_path(profile_dir) {
        let _ = client.hsts().save_to(&path);
    }
    if let Some(dir) = profile_dir {
        if let Err(error) = client.cookies.save(&dir.join("cookies.json")) {
            eprintln!("browser: cookies could not save: {error}");
        }
    }
}

enum Navigation {
    Document(net::Fetched),
    Download(crate::downloads::Source),
}
async fn fetch_navigation(
    client: &Client,
    url: &Url,
    allow_downgrade: bool,
    cache: net::navigation::CacheMode,
    on_chunk: &mut (dyn FnMut(&net::ResponseHead, &[u8], Progress) + Send),
) -> Result<Navigation, Error> {
    let mut headers = Vec::new();
    match cache {
        net::navigation::CacheMode::Normal => {}
        net::navigation::CacheMode::Revalidate => {
            headers.push(("Cache-Control".into(), "max-age=0".into()))
        }
        net::navigation::CacheMode::Bypass => {
            headers.push(("Cache-Control".into(), "no-cache, no-store".into()));
            headers.push(("Pragma".into(), "no-cache".into()));
        }
    }
    let timeout = std::time::Duration::from_secs(60);
    let deadline = tokio::time::Instant::now() + timeout;
    let opened = client.open_navigation(url, headers.clone(), timeout).await;
    let (head, mut body) = match opened {
        Err(error @ (Error::Connect(_) | Error::Dns(_) | Error::Timeout(_)))
            if allow_downgrade && url.scheme() == "https" =>
        {
            let mut plain = url.clone();
            let _ = plain.set_scheme("http");
            if client.resource_url(&plain)?.scheme() == "https" {
                return Err(error);
            }
            client
                .open_navigation(
                    &plain,
                    headers,
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                )
                .await?
        }
        other => other?,
    };
    if !(200..300).contains(&head.status) {
        return Err(Error::HttpStatus(head.status));
    }
    let content_type = head.header("content-type").map(str::to_owned);
    let mime = content_type
        .as_deref()
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let attachment = head.header("content-disposition").is_some_and(|s| {
        s.split(';')
            .next()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("attachment"))
    });
    let image = mime.starts_with("image/")
        && matches!(
            mime.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/svg+xml"
        );
    let html = mime.is_empty() || mime == "text/html" || mime == "application/xhtml+xml";
    let text = mime.starts_with("text/")
        || matches!(mime.as_str(), "application/json" | "application/xml");
    if attachment || !html && !text && !image {
        let filename =
            crate::downloads::suggested_filename(&head.url, head.header("content-disposition"));
        return Ok(Navigation::Download(crate::downloads::Source {
            force_prompt: false,
            url: head.url,
            top: url.clone(),
            filename,
            bytes: None,
        }));
    }
    let policies = head
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-security-policy"))
        .map(|(_, v)| v.clone())
        .collect::<Vec<_>>();
    let response = net::ResponseHead {
        url: head.url.clone(),
        status: head.status,
        content_type: content_type.clone(),
        script_policies: policies.clone(),
    };
    let total = head.header("content-length").and_then(|s| s.parse().ok());
    on_chunk(
        &response,
        &[],
        Progress {
            downloaded: 0,
            total,
        },
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout_at(deadline, body.next())
        .await
        .map_err(|_| Error::Timeout("Document body deadline".into()))?
    {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > client.document_byte_limit() {
            return Err(Error::BodyTooLarge {
                cap: client.document_byte_limit(),
            });
        }
        bytes.extend_from_slice(&chunk);
        if html {
            on_chunk(
                &response,
                &chunk,
                Progress {
                    downloaded: bytes.len(),
                    total,
                },
            );
        }
    }
    let mut policies = policies;
    let content_type = if !html {
        policies.push("script-src 'none'".into());
        if image {
            bytes = format!(
                "<!doctype html><title>Image</title><img alt=\"Image\" src=\"{}\">",
                crate::file_ops::escape(head.url.as_str())
            )
            .into_bytes();
        } else {
            bytes = format!(
                "<!doctype html><meta charset=utf-8><title>Text document</title><pre>{}</pre>",
                crate::file_ops::escape(&String::from_utf8_lossy(&bytes))
            )
            .into_bytes();
        }
        Some("text/html; charset=utf-8".into())
    } else {
        content_type
    };
    Ok(Navigation::Document(net::Fetched {
        url: head.url,
        status: head.status,
        content_type,
        bytes,
        script_policies: policies,
    }))
}

/// Resolve omnibox text to a navigation target.
///
/// Returns `Ok((url, allow_downgrade))` for URLs (`allow_downgrade` is true
/// only for bare-host input per plan/04 §4.3.3) or the search query string.
pub fn resolve_input(text: &str) -> Result<(Url, bool), String> {
    match net::classify_user_input(text) {
        UserInput::Url(url) => {
            let bare_host = !text.trim().contains("://");
            Ok((url, bare_host))
        }
        UserInput::Search(query) => Err(query),
    }
}
