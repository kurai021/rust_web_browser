//! Graphical navigation, progressive HTML/CSS and Stage A painting (plan/10).
//!
//! Toolbar with back/forward/stop-reload buttons, an editable omnibox, a
//! flow viewport and status bar. Network/decode/layout stay off the UI thread;
//! page-owned assets and revision IDs prevent stale navigation/resize results.

pub mod app;
pub mod article;
pub mod fetcher;
pub mod fonts;
pub mod history;
pub mod images;
pub mod layout_worker;
pub mod omnibox;
pub mod page;
pub mod text;
pub mod viewport;

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
    /// Skip GPU initialization and use the same display list on softbuffer.
    pub software_render: bool,
    /// Emit local layout/frame counters; no telemetry or network reporting.
    pub perf: bool,
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
    let fetcher = Fetcher::spawn(
        options.fetch.clone(),
        options.profile_dir.clone(),
        wake.clone(),
    );
    let mut app = ShellApp::new(
        fetcher,
        options.start_url,
        options.start_allow_downgrade,
        options.start_search,
    );
    app.configure_rendering(options.software_render, options.perf, wake);
    let _ = event_loop.run_app(&mut app);
    if let Some(fatal) = app.take_fatal() {
        app.shutdown();
        return Err(ShellError::WindowInit(fatal));
    }
    app.shutdown();
    Ok(())
}
