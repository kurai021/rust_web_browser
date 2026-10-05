//! Background fetcher (plan/04 §4.3.2, plan/03 §3.3).
//!
//! Network I/O never runs on the UI thread: a dedicated worker thread owns
//! a `tokio` runtime and an `net::Client`. The UI sends [`Command`]s and
//! drains [`FetchEvent`]s (polled in `about_to_wait`). A new navigation or
//! an explicit stop aborts the in-flight task — dropping the future cancels
//! the request (plan/04 cancellation rule).

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::fonts::{load_fonts, FontAsset};
use crate::page::{load_page, Page};
use css::Environment;
use net::{Client, Error, FetchOptions, Progress, UserInput};
use tokio::task::JoinHandle as TokioHandle;
use url::Url;

/// Commands from the UI thread to the worker.
#[derive(Debug)]
pub enum Command {
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
    /// A fetch started (progress bar on).
    Started { id: u64, url: Url },
    /// Body progress update.
    Progress {
        id: u64,
        downloaded: usize,
        total: Option<u64>,
    },
    /// Fetch finished (success or typed failure).
    Done {
        id: u64,
        url: Url,
        result: Result<(Arc<Page>, css::ComputedStyles), Error>,
    },
    /// Font-display swap: fonts arrive after the styled fallback document.
    Fonts { id: u64, assets: Vec<FontAsset> },
    Styled {
        id: u64,
        revision: u64,
        styles: css::ComputedStyles,
    },
    /// Nothing in flight anymore after a stop.
    Stopped { id: u64 },
}

/// Handle held by the UI thread.
pub struct Fetcher {
    commands: Sender<Command>,
    events: Receiver<FetchEvent>,
    worker: Option<JoinHandle<()>>,
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
        let worker = std::thread::Builder::new()
            .name("net-worker".to_owned())
            .spawn(move || worker_main(commands_rx, events_tx, wake, options, profile_dir))
            .expect("net worker thread spawns");
        Self {
            commands: commands_tx,
            events: events_rx,
            worker: Some(worker),
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
        let _ = self.commands.send(command);
    }

    /// Stop the worker thread (also aborts any in-flight fetch).
    pub fn shutdown(mut self) {
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
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("net-io")
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    let client = Arc::new(Client::new(options).expect("TLS stack initializes"));
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
                in_flight = Some(runtime.spawn(async move {
                    let _ = events.send(FetchEvent::Started {
                        id,
                        url: url.clone(),
                    });
                    wake();
                    let events_for_progress = events.clone();
                    let mut last_report = 0usize;
                    let result = client
                        .fetch_with_http_fallback(
                            &url,
                            allow_downgrade,
                            &mut |progress: Progress| {
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
                    let result = match result {
                        Ok(fetched) => {
                            let page = Arc::new(load_page(&client, fetched).await);
                            let styles = page.computed_styles(environment);
                            Ok((page, styles))
                        }
                        Err(err) => Err(err),
                    };
                    let font_page = result.as_ref().ok().map(|(page, _)| page.clone());
                    persist_hsts(&client, &profile_dir);
                    let _ = events.send(FetchEvent::Done { id, url, result });
                    wake();
                    if let Some(page) = font_page {
                        let assets = load_fonts(&client, &page, environment).await;
                        if !assets.is_empty() {
                            let _ = events.send(FetchEvent::Fonts { id, assets });
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
                        styles,
                    });
                    wake();
                    let assets = load_fonts(&client, &page, environment).await;
                    if !assets.is_empty() {
                        let _ = events.send(FetchEvent::Fonts { id, assets });
                        wake();
                    }
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
