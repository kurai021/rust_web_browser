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
use net::{Client, Error, FetchOptions, Progress, UserInput};
use tokio::task::JoinHandle as TokioHandle;
use url::Url;

/// Commands from the UI thread to the worker.
#[derive(Debug)]
pub enum Command {
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
    Stop { id: u64 },
    /// Persist HSTS under `dir` and exit the worker.
    Shutdown,
}

/// Events from the worker to the UI thread.
#[derive(Debug)]
pub enum FetchEvent {
    ScriptPage {
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
        let (commands_tx, commands_rx) = channel::<Command>();
        let (events_tx, events_rx) = channel::<FetchEvent>();
        let active_script = Arc::new(AtomicU64::new(0));
        let token = active_script.clone();
        let worker = std::thread::Builder::new()
            .name("net-worker".to_owned())
            .spawn(move || worker_main(commands_rx, events_tx, wake, options, profile_dir, token))
            .expect("net worker thread spawns");
        Self {
            commands: commands_tx,
            events: events_rx,
            worker: Some(worker),
            active_script,
        }
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
            Command::Navigate { id, .. } => {
                self.active_script.store(*id, Ordering::Relaxed);
            }
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

fn hsts_path(profile_dir: &Option<PathBuf>) -> Option<PathBuf> {
    profile_dir.as_ref().map(|dir| dir.join("hsts.json"))
}

fn worker_main(
    commands: Receiver<Command>,
    events: Sender<FetchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    options: FetchOptions,
    profile_dir: Option<PathBuf>,
    active_script: Arc<AtomicU64>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .thread_name("net-io")
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    let client = Arc::new(Client::new(options).expect("TLS stack initializes"));
    let scripts = match crate::script_worker::ScriptWorker::spawn(
        client.clone(),
        events.clone(),
        wake.clone(),
        active_script,
    ) {
        Ok(worker) => Arc::new(std::sync::Mutex::new(worker)),
        Err(error) => {
            eprintln!("browser: script worker failed: {error}");
            return;
        }
    };
    if let Some(path) = hsts_path(&profile_dir) {
        let loaded = net::HstsStore::load_from(&path);
        *client.hsts() = loaded;
    }
    let mut in_flight: Option<TokioHandle<()>> = None;
    let mut restyling: Option<TokioHandle<()>> = None;

    let abort_in_flight = |slot: &mut Option<TokioHandle<()>>| {
        if let Some(handle) = slot.take() {
            handle.abort();
        }
    };

    while let Ok(command) = commands.recv() {
        match command {
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
                let profile_dir = profile_dir.clone();
                let scripts = scripts.clone();
                in_flight = Some(runtime.spawn(async move {
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
                    let result = client
                        .fetch_stream_with_http_fallback(
                            &url,
                            allow_downgrade,
                            &mut |head, chunk, progress: Progress| {
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
                    persist_hsts(&client, &profile_dir);
                    match result {
                        Ok(fetched) => scripts
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .initialize(id, fetched, environment),
                        Err(error) => {
                            let _ = events.send(FetchEvent::Done {
                                id,
                                url,
                                result: Err(error),
                            });
                            wake();
                        }
                    }
                }));
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
                restyling = Some(runtime.spawn(async move {
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
                }));
            }
            Command::Stop { id } => {
                abort_in_flight(&mut in_flight);
                abort_in_flight(&mut restyling);
                let _ = events.send(FetchEvent::Stopped { id });
                wake();
            }
            Command::Shutdown => {
                abort_in_flight(&mut in_flight);
                abort_in_flight(&mut restyling);
                persist_hsts(&client, &profile_dir);
                scripts.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
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
