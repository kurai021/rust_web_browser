//! `shell` — graphical navigation and Phase 3 HTML/CSS integration (plan/10).
//!
//! Toolbar with back/forward/stop-reload buttons, an editable omnibox, a
//! scrollable raw-source viewport and a status bar. The real page viewport
//! (`paint`) and multi-tab UI land in later phases; the window, input and
//! background-fetch plumbing built here are reused by them.

pub mod app;
pub mod article;
pub mod fetcher;
pub mod fonts;
pub mod history;
pub mod omnibox;
pub mod page;
pub mod text;

use std::path::PathBuf;
use std::sync::Arc;

use net::FetchOptions;
use thiserror::Error;
use url::Url;
use winit::event_loop::EventLoop;

use app::{ShellApp, Wake};
use fetcher::Fetcher;

/// Startup configuration for the window.
#[derive(Debug, Clone)]
pub struct Options {
    /// Initial navigation target (already classified).
    pub start_url: Option<Url>,
    /// Whether the https→http fallback applies to `start_url`.
    pub start_allow_downgrade: bool,
    /// Startup search query (shown as a stub until Phase 9).
    pub start_search: Option<String>,
    /// Fetch tunables (plan/04 §4.3.2).
    pub fetch: FetchOptions,
    /// Profile dir for HSTS persistence (`None` = memory only).
    pub profile_dir: Option<PathBuf>,
}

/// Window startup failure.
#[derive(Debug, Error)]
pub enum ShellError {
    /// No display / event loop unavailable.
    #[error("event loop failed: {0}")]
    EventLoop(String),
    /// Window or software surface failed.
    #[error("window init failed: {0}")]
    WindowInit(String),
}

/// Open the window and run it until closed. Blocks the calling thread.
pub fn run(options: Options) -> Result<(), ShellError> {
    let event_loop = EventLoop::<Wake>::with_user_event()
        .build()
        .map_err(|e| ShellError::EventLoop(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = proxy.send_event(Wake::Fetch);
    });
    let fetcher = Fetcher::spawn(options.fetch.clone(), options.profile_dir.clone(), wake);
    let mut app = ShellApp::new(
        fetcher,
        options.start_url,
        options.start_allow_downgrade,
        options.start_search,
    );
    let _ = event_loop.run_app(&mut app);
    if let Some(fatal) = app.take_fatal() {
        app.shutdown();
        return Err(ShellError::WindowInit(fatal));
    }
    app.shutdown();
    Ok(())
}
