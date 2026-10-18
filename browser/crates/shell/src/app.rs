//! Phase 7 window: dynamic pages, native permission chrome and retained painting.
//!
//! winit chrome, wgpu compositing and the matching softbuffer fallback.
//! Network, image decoding, page layout and glyph preparation stay on workers.

use std::num::NonZeroU32;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::page::Page;
use css::{ComputedStyles, Environment};
use net::Error as NetError;
use softbuffer::{Context, Surface};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes};

use crate::article::{Article, Block, BlockView};
use crate::fetcher::{resolve_input, Command, FetchEvent, Fetcher};
use crate::history::History;
use crate::omnibox::Omnibox;
use crate::text::{rgb, Canvas, TextBuffer, TextRenderer};
mod tabs;
use tabs::{Presentation, TabState};
mod browser_ui;
#[cfg(test)]
mod phase9_tests;
use browser_ui::{Action as ChromeAction, BrowserUi, LocalPage};

/// Wake signal from the network worker.
#[derive(Debug)]
pub enum Wake {
    /// New fetch events are queued; drain them.
    Fetch,
}

/// Layout metrics (physical pixels).
const TABS_H: i32 = 32;
const NAV_H: i32 = 44;
const BOOKMARKS_H: i32 = 28;
const TOOLBAR_H: i32 = TABS_H + NAV_H + BOOKMARKS_H;
const FIND_H: i32 = 40;
const STATUS_H: i32 = 26;
const BUTTON_W: i32 = 34;
const BUTTON_GAP: i32 = 6;
const PAD: i32 = 8;
const SCROLLBAR_W: i32 = 12;
const LINE_SCROLL_PX: f32 = 60.0;

const BG: u32 = rgb(0xF2, 0xF2, 0xF2);
const CHROME_BG: u32 = rgb(0xE4, 0xE4, 0xE4);
const VIEW_BG: u32 = rgb(0xFF, 0xFF, 0xFF);
const ACCENT: u32 = rgb(0x0B, 0x5D, 0xC2);
const BORDER: u32 = rgb(0xC8, 0xC8, 0xC8);
const TRACK: u32 = rgb(0xD8, 0xD8, 0xD8);
const THUMB: u32 = rgb(0xA0, 0xA0, 0xA0);
const SELECT_BG: u32 = rgb(0xBB, 0xD7, 0xFF);

/// Built-in welcome article (also serves for `about:blank`).
fn welcome_article() -> Article {
    Article {
        title: Some("Welcome".to_owned()),
        blocks: vec![
            Block::Heading { level: 1, text: "Your browser — Phase 9".to_owned(), links: Vec::new() },
            Block::Paragraph {
                text: "Tabs, saved sessions, local history and bookmarks. Downloads stream to disk with pause and resume. Pages use the in-house HTML/CSS/JavaScript engines and GPU/software painting.".to_owned(),
                links: Vec::new(),
            },
            Block::Heading { level: 2, text: "Try".to_owned(), links: Vec::new() },
            Block::ListItem { depth: 1, text: "example.com".to_owned(), links: Vec::new() },
            Block::ListItem { depth: 1, text: "https://example.com/".to_owned(), links: Vec::new() },
            Block::ListItem { depth: 1, text: "about:blank (this page)".to_owned(), links: Vec::new() },
            Block::Heading { level: 2, text: "Shortcuts".to_owned(), links: Vec::new() },
            Block::Paragraph {
                text: "Ctrl+T/W opens/closes a tab. Ctrl+Tab switches tabs. Ctrl+L focuses the address bar; Enter navigates or searches. Ctrl+H/B/J opens history/bookmarks/downloads. Ctrl+F finds text, Ctrl++/- zooms, Ctrl+P prints to PDF. Alt+Left/Right navigates history. Settings: about:preferences.".to_owned(),
                links: Vec::new(),
            },
        ],
        ..Article::default()
    }
}

/// What the viewport currently shows (status semantics; the pixels come
/// from `article` + `blocks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Welcome,
    Document,
    Error,
}

/// Loading progress mirrored from worker events.
#[derive(Debug, Clone)]
enum LoadState {
    Idle,
    Loading {
        url: Url,
        downloaded: usize,
        total: Option<u64>,
    },
}

struct Button {
    label: &'static str,
    rect: (i32, i32, i32, i32),
    enabled: bool,
}

/// The running window and all of its UI state.
pub struct ShellApp {
    browser: BrowserUi,
    local_page: Option<LocalPage>,
    batch_visible: Option<usize>,
    session_dirty: bool,
    next_cache: net::navigation::CacheMode,
    previous_navigation: Option<(u64, bool)>,
    previous_presentation: Option<Presentation>,
    committed: bool,
    pending_traversal: i32,
    page_font_db: cosmic_text::fontdb::Database,
    navigation_url: Option<Url>,
    crashed: bool,
    favicon: Option<Arc<layout::RasterImage>>,
    tabs: Vec<Option<TabState>>,
    active_tab: usize,
    next_navigation_id: u64,
    shared: Arc<crate::fetcher::Shared>,
    profile: profile::Store,
    exit_requested: bool,
    handling_background: bool,
    restore_session: bool,
    pending_restore: Option<profile::SavedTab>,
    restore_scroll: Option<(f32, f32)>,
    fetcher: Option<Fetcher>,
    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    gpu: Option<paint::gpu::GpuWindow>,
    gpu_pending: Option<Receiver<Result<paint::gpu::GpuWindow, String>>>,
    gpu_worker: Option<std::thread::JoinHandle<()>>,
    gpu_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    software_render: bool,
    perf: bool,
    pixels: Vec<u32>,
    size: PhysicalSize<u32>,
    fatal_init: Option<String>,
    last_window_title: String,

    font_system: cosmic_text::FontSystem,
    text_renderer: TextRenderer,
    omnibox_text: TextBuffer,
    omnibox_prefix: TextBuffer,
    status_text: TextBuffer,

    history: History,
    omnibox: Omnibox,
    omnibox_focused: bool,
    view: View,
    article: Article,
    page: Option<Arc<Page>>,
    redirected_from: Option<Url>,
    styles: ComputedStyles,
    navigation_id: u64,
    style_revision: u64,
    dark_theme: bool,
    web_font_ids: Vec<cosmic_text::fontdb::ID>,
    page_origin: (f32, f32),
    canvas_color: css::values::Color,
    blocks: Vec<BlockView>,
    flow: Option<Arc<layout::LayoutResult>>,
    scene: paint::Scene,
    layout_worker: Option<crate::layout_worker::LayoutWorker>,
    font_generation: u64,
    flow_navigation: Option<u64>,
    scroll_offsets: layout::ScrollOffsets,
    zoom: f32,
    layout_passes: u64,
    layout_ms: f64,
    frames: u64,
    navigation_started: Option<std::time::Instant>,
    painted_navigation: bool,
    preview_seen: bool,
    script_ready: bool,
    dom_focused: Option<html::NodeId>,
    script_dialog: Option<String>,
    script_console: Vec<String>,
    web_dialog: Option<crate::web_ui::WebDialog>,
    web_dialogs: std::collections::VecDeque<crate::web_ui::WebDialog>,
    spa_back: bool,
    spa_forward: bool,
    observer_viewport: Option<web_api::runtime::GeometryRect>,
    load: LoadState,
    pending_history: bool,
    scroll_y: f32,
    scroll_x: f32,
    content_h: f32,
    content_measured_for: Option<(u64, u32)>,
    content_version: u64,

    mouse_x: f64,
    mouse_y: f64,
    modifiers: ModifiersState,
    dragging_scroll: bool,
    start_url: Option<Url>,
    start_allow_downgrade: bool,
    start_search: Option<String>,
}

impl ShellApp {
    /// Build UI state. The window itself is created in `resumed`.
    pub fn new(
        fetcher: Fetcher,
        start_url: Option<Url>,
        start_allow_downgrade: bool,
        start_search: Option<String>,
    ) -> Self {
        let mut font_system = cosmic_text::FontSystem::new();
        let text_renderer = TextRenderer::new();
        let omnibox_text =
            TextBuffer::with_metrics(&mut font_system, 15.0, cosmic_text::Wrap::None);
        let omnibox_prefix =
            TextBuffer::with_metrics(&mut font_system, 15.0, cosmic_text::Wrap::None);
        let status_text = TextBuffer::with_metrics(&mut font_system, 13.0, cosmic_text::Wrap::None);
        let page_font_db = font_system.db().clone();
        let mut app = Self {
            browser: BrowserUi::default(),
            local_page: None,
            batch_visible: None,
            session_dirty: false,
            next_cache: Default::default(),
            previous_navigation: None,
            previous_presentation: None,
            committed: false,
            pending_traversal: 0,
            page_font_db,
            navigation_url: None,
            crashed: false,
            favicon: None,
            tabs: vec![None],
            active_tab: 0,
            next_navigation_id: 0,
            shared: fetcher.shared(),
            profile: profile::Store::open(None).expect("in-memory defaults"),
            exit_requested: false,
            handling_background: false,
            restore_session: true,
            pending_restore: None,
            restore_scroll: None,
            fetcher: Some(fetcher),
            window: None,
            context: None,
            surface: None,
            gpu: None,
            gpu_pending: None,
            gpu_worker: None,
            gpu_wake: None,
            software_render: false,
            perf: false,
            pixels: Vec::new(),
            size: PhysicalSize::new(1100, 750),
            fatal_init: None,
            last_window_title: String::new(),
            font_system,
            text_renderer,
            omnibox_text,
            omnibox_prefix,
            status_text,
            history: History::new(),
            omnibox: Omnibox::new(),
            omnibox_focused: true,
            view: View::Welcome,
            article: welcome_article(),
            page: None,
            redirected_from: None,
            styles: ComputedStyles::default(),
            navigation_id: 0,
            style_revision: 0,
            dark_theme: false,
            web_font_ids: Vec::new(),
            page_origin: (12.0, 10.0),
            canvas_color: css::values::Color::WHITE,
            blocks: Vec::new(),
            flow: None,
            scene: paint::Scene::default(),
            layout_worker: None,
            font_generation: 0,
            flow_navigation: None,
            scroll_offsets: layout::ScrollOffsets::default(),
            zoom: 1.0,
            layout_passes: 0,
            layout_ms: 0.0,
            frames: 0,
            navigation_started: None,
            painted_navigation: false,
            preview_seen: false,
            script_ready: false,
            dom_focused: None,
            script_dialog: None,
            script_console: Vec::new(),
            web_dialog: None,
            web_dialogs: std::collections::VecDeque::new(),
            spa_back: false,
            spa_forward: false,
            observer_viewport: None,
            load: LoadState::Idle,
            pending_history: false,
            scroll_y: 0.0,
            scroll_x: 0.0,
            content_h: 0.0,
            content_measured_for: None,
            content_version: 0,
            mouse_x: 0.0,
            mouse_y: 0.0,
            modifiers: ModifiersState::empty(),
            dragging_scroll: false,
            start_url,
            start_allow_downgrade,
            start_search,
        };
        app.set_article(welcome_article(), View::Welcome);
        app
    }

    /// Take the startup error, if window/surface creation failed.
    pub fn take_fatal(&mut self) -> Option<String> {
        self.fatal_init.take()
    }

    pub fn configure_rendering(
        &mut self,
        software: bool,
        perf: bool,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.software_render = software;
        self.perf = perf;
        match crate::layout_worker::LayoutWorker::spawn(wake.clone()) {
            Ok(worker) => self.layout_worker = Some(worker),
            Err(err) => self.fatal_init = Some(format!("layout worker failed: {err}")),
        }
        self.gpu_wake = Some(wake);
        let wake = self.gpu_wake.clone().unwrap_or_else(|| Arc::new(|| {}));
        match crate::find::Worker::spawn(wake.clone()) {
            Ok(worker) => self.browser.find_worker = Some(worker),
            Err(error) => self.browser.notice = error.to_string(),
        }
        match crate::downloads::Manager::spawn(
            self.shared.client.clone(),
            self.profile.data.downloads.clone(),
            wake,
        ) {
            Ok(manager) => self.browser.downloads = Some(manager),
            Err(error) => self.browser.notice = error.to_string(),
        }
    }

    fn start_gpu(&mut self, window: Arc<Window>) {
        if self.software_render {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = self.gpu_wake.clone();
        match std::thread::Builder::new()
            .name("paint-init".into())
            .spawn(move || {
                let size = window.inner_size();
                let result = pollster::block_on(paint::gpu::GpuWindow::new(
                    window,
                    layout::Size::new(size.width as f32, size.height as f32),
                ));
                let _ = tx.send(result);
                if let Some(wake) = wake {
                    wake();
                }
            }) {
            Ok(worker) => {
                self.gpu_worker = Some(worker);
                self.gpu_pending = Some(rx);
            }
            Err(err) => {
                eprintln!("browser: GPU init unavailable, using software: {err}");
                self.software_render = true;
            }
        }
    }

    fn poll_gpu(&mut self) {
        let ready = self.gpu_pending.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = ready {
            self.gpu_pending = None;
            match result {
                Ok(gpu) => {
                    if self.perf {
                        eprintln!("PERF renderer=gpu adapter={}", gpu.gpu.adapter_name);
                    }
                    self.surface = None;
                    self.context = None;
                    self.gpu = Some(gpu);
                }
                Err(err) => {
                    eprintln!("browser: GPU unavailable, using software: {err}");
                    self.software_render = true;
                }
            }
            self.request_redraw();
        }
    }

    /// Join the worker thread (saves HSTS). Called after the loop exits.
    pub fn shutdown(&mut self) {
        if let Some(manager) = &mut self.browser.downloads {
            manager.shutdown();
        }
        self.drain_browser();
        self.save_session();
        for tab in self.tabs.iter_mut().flatten() {
            if let Some(fetcher) = tab.fetcher.take() {
                fetcher.shutdown();
            }
        }
        if let Some(worker) = &mut self.layout_worker {
            worker.shutdown();
        }
        if let Some(fetcher) = self.fetcher.take() {
            fetcher.shutdown();
        }
        if let Some(worker) = self.gpu_worker.take() {
            let _ = worker.join();
        }
        self.profile.shutdown();
    }

    pub fn configure_profile(&mut self, profile: profile::Store, restore: bool) {
        self.restore_session = restore;
        self.dark_theme = profile.preferences.appearance.theme == "dark";
        self.profile = profile;
    }
    /// The window and headless acceptance harness share this startup path.
    pub fn initialize_navigation(&mut self) {
        self.restore_startup();
    }
    /// Service channels without a window; no transport/VM/layout runs on this caller.
    pub fn pump(&mut self) {
        self.drain_fetcher();
        self.drain_layout();
        self.drain_browser();
        self.measure_content(self.size.width as f32);
    }
    pub fn navigate(&mut self, input: &str) {
        self.omnibox_focused = false;
        self.navigate_input(input);
    }
    pub fn snapshot(&self) -> Option<Arc<Page>> {
        self.page.clone()
    }
    pub fn current_url(&self) -> Option<&Url> {
        self.history.current()
    }
    pub fn is_loading(&self) -> bool {
        matches!(self.load, LoadState::Loading { .. })
    }
    pub fn tab_crashed(&self) -> bool {
        self.crashed
    }
    pub fn back(&mut self) {
        self.go_back();
    }
    pub fn forward(&mut self) {
        self.go_forward();
    }
    pub fn click_node(&self, node: html::NodeId) {
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::DomEvent {
                id: self.navigation_id,
                event: crate::script_worker::DomCommand::Click(node),
            });
        }
    }

    // ---------- navigation ----------

    fn submit_omnibox(&mut self) {
        if let Some(index) = self.browser.suggestion.take() {
            if let Some((url, _)) = self
                .profile
                .data
                .suggestions(self.omnibox.text(), 6)
                .get(index)
            {
                self.omnibox.set_text(url);
            }
        }
        let input = self.omnibox.text().to_owned();
        self.omnibox_focused = false;
        self.navigate_input(&input);
    }

    fn navigate_input(&mut self, input: &str) {
        let trimmed = input.trim();
        let local = match trimmed.to_ascii_lowercase().as_str() {
            "about:preferences" | "about:config" => Some(LocalPage::Preferences),
            "about:history" => Some(LocalPage::History),
            "about:bookmarks" => Some(LocalPage::Bookmarks),
            "about:downloads" => Some(LocalPage::Downloads),
            "about:devtools" => Some(LocalPage::Devtools),
            _ => None,
        };
        if let Some(page) = local {
            self.open_local(page);
            return;
        }
        if trimmed.eq_ignore_ascii_case("about:permissions") {
            let mut dialog=crate::web_ui::WebDialog::notice("Site permissions are origin-scoped. Revoke saved and current grants here. Camera and microphone have no backend and are denied.");
            dialog.origin = self.page.as_ref().map_or_else(
                || "Browser permissions".into(),
                |p| p.url.origin().ascii_serialization(),
            );
            dialog.kind = crate::web_ui::Kind::Settings;
            self.queue_web_dialog(dialog);
            return;
        }
        if trimmed.eq_ignore_ascii_case("about:blank") || trimmed.is_empty() {
            self.show_welcome();
            return;
        }
        if trimmed.to_ascii_lowercase().starts_with("about:") {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::Stop {
                    id: self.navigation_id,
                });
            }
            self.previous_navigation = None;
            self.previous_presentation = None;
            self.navigation_id = self.next_navigation();
            self.script_ready = false;
            self.load = LoadState::Idle;
            self.local_page = None;
            let title = if trimmed.starts_with("about:cert-error") {
                "Certificate error"
            } else if trimmed.starts_with("about:net-error") {
                "Network error"
            } else {
                "Local browser page"
            };
            self.set_article(Article{title:Some(title.into()),blocks:vec![Block::Heading{level:1,text:title.into(),links:Vec::new()},Block::Paragraph{text:"This page is generated locally. Use about:preferences, about:history, about:bookmarks or about:downloads. Network and certificate failures show their details here without remote assets.".into(),links:Vec::new()}],..Default::default()},View::Error);
            self.omnibox.set_text(trimmed);
            if let Ok(url) = Url::parse(trimmed) {
                if self.history.current() != Some(&url) {
                    self.history.navigate(url);
                }
            }
            self.navigation_url = None;
            self.save_session();
            self.update_title();
            self.request_redraw();
            return;
        }
        match resolve_input(trimmed) {
            Ok((url, allow_downgrade)) => self.start_fetch(url, allow_downgrade, true),
            Err(query) => match self.profile.preferences.search_url(&query) {
                Ok(url) => self.start_fetch(url, false, true),
                Err(error) => self.browser.notice = error,
            },
        }
    }

    /// Begin a fetch. `record_history` is false for back/forward/reload
    /// (the entry already exists); the final URL is recorded on Done.
    fn start_fetch(&mut self, url: Url, allow_downgrade: bool, record_history: bool) {
        if self.crashed {
            if let Some(fetcher) = self.fetcher.take() {
                fetcher.shutdown();
            }
            let wake = self.gpu_wake.clone().unwrap_or_else(|| Arc::new(|| {}));
            match Fetcher::spawn_shared(self.shared.clone(), wake) {
                Ok(fetcher) => {
                    self.fetcher = Some(fetcher);
                    self.crashed = false;
                }
                Err(error) => {
                    self.browser.notice = format!("Cannot restart tab: {error}");
                    return;
                }
            }
        }
        if !matches!(self.load, LoadState::Loading { .. }) {
            self.previous_navigation = Some((self.navigation_id, self.script_ready));
            self.previous_presentation = Some(Presentation::capture(self));
        }
        self.navigation_url = Some(url.clone());
        self.committed = false;
        self.pending_traversal = 0;
        self.local_page = None;
        self.navigation_id = self.next_navigation();
        self.style_revision = self.style_revision.wrapping_add(1);
        self.pending_history = record_history;
        self.navigation_started = Some(std::time::Instant::now());
        self.painted_navigation = false;
        self.preview_seen = false;
        self.script_ready = false;
        self.dom_focused = None;
        self.script_dialog = None;
        self.script_console.clear();
        self.web_dialog = None;
        self.web_dialogs.clear();
        self.spa_back = false;
        self.spa_forward = false;
        self.observer_viewport = None;
        self.load = LoadState::Loading {
            url: url.clone(),
            downloaded: 0,
            total: None,
        };
        self.update_title();
        self.request_redraw();
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::NavigationPolicy {
                javascript: self.profile.preferences.privacy.javascript_enabled,
                sites: self.profile.preferences.privacy.site_javascript.clone(),
                cache: std::mem::take(&mut self.next_cache),
            });
            fetcher.send(Command::Navigate {
                id: self.navigation_id,
                url,
                allow_downgrade,
                environment: self.environment(),
            });
        }
    }

    fn reload(&mut self) {
        self.reload_mode(net::navigation::CacheMode::Revalidate);
    }

    fn reload_mode(&mut self, mode: net::navigation::CacheMode) {
        self.next_cache = mode;
        if let Some(url) = self
            .navigation_url
            .clone()
            .or_else(|| self.history.current().cloned())
        {
            self.navigate_saved(url.as_str());
        } else if matches!(self.view, View::Error) {
            self.show_welcome();
        }
    }

    fn go_back(&mut self) {
        if self.spa_back && self.script_ready {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::DomEvent {
                    id: self.navigation_id,
                    event: crate::script_worker::DomCommand::HistoryGo(-1),
                });
            }
            return;
        }
        if let Some(url) = self.history.back_target() {
            self.omnibox.set_text(url.as_str());
            if url.scheme() == "about" {
                self.history.go_back();
                self.navigate_saved(url.as_str());
                return;
            }
            self.navigate_saved(url.as_str());
            self.pending_traversal = -1;
        }
    }

    fn go_forward(&mut self) {
        if self.spa_forward && self.script_ready {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::DomEvent {
                    id: self.navigation_id,
                    event: crate::script_worker::DomCommand::HistoryGo(1),
                });
            }
            return;
        }
        if let Some(url) = self.history.forward_target() {
            self.omnibox.set_text(url.as_str());
            if url.scheme() == "about" {
                self.history.go_forward();
                self.navigate_saved(url.as_str());
                return;
            }
            self.navigate_saved(url.as_str());
            self.pending_traversal = 1;
        }
    }

    fn stop_or_reload(&mut self) {
        if matches!(self.load, LoadState::Loading { .. }) {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(if self.committed {
                    Command::Stop {
                        id: self.navigation_id,
                    }
                } else {
                    Command::StopNavigation {
                        id: self.navigation_id,
                    }
                });
            }
        } else {
            self.reload();
        }
    }
    pub fn stop(&mut self) {
        if matches!(self.load, LoadState::Loading { .. }) {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(if self.committed {
                    Command::Stop {
                        id: self.navigation_id,
                    }
                } else {
                    Command::StopNavigation {
                        id: self.navigation_id,
                    }
                });
            }
        }
    }

    fn bump_content(&mut self) {
        self.content_version += 1;
        self.scroll_y = 0.0;
    }

    /// Swap the article, reset scroll and schedule a rebuild.
    fn set_article(&mut self, article: Article, view: View) {
        if view != View::Document {
            self.script_ready = false;
            self.spa_back = false;
            self.spa_forward = false;
            self.dom_focused = None;
            self.clear_web_fonts();
            self.page = None;
            self.canvas_color = css::values::Color::WHITE;
            self.flow = None;
            self.flow_navigation = None;
            self.scene = paint::Scene::default();
            self.scroll_offsets.clear();
            self.zoom = 1.0;
            self.scroll_x = 0.0;
        }
        self.view = view;
        self.article = article;
        self.bump_content();
    }

    /// Rebuild laid-out blocks at `width` px.
    fn rebuild_blocks(&mut self, width: f32) {
        if let Some(page) = &self.page {
            if let Some(worker) = &self.layout_worker {
                worker.request(crate::layout_worker::Request {
                    navigation: self.navigation_id,
                    revision: self.content_version,
                    viewport: layout::Size::new(width, self.viewport_h()),
                    page: page.clone(),
                    styles: self.styles.clone(),
                    font_generation: self.font_generation,
                    locale: self.font_system.locale().to_owned(),
                    db: self.page_font_db.clone(),
                });
            }
            return;
        }
        let env = self.environment();
        let mut content_width = width;
        self.page_origin = (12.0, 10.0);
        if let Some(style) = &self.article.page_style {
            let ctx = env.length_context(style.font_size, 16.0, width);
            let margin: [f32; 4] =
                std::array::from_fn(|i| style.margin[i].resolve(ctx).unwrap_or(0.0));
            let padding: [f32; 4] =
                std::array::from_fn(|i| style.padding[i].resolve(ctx).unwrap_or(0.0).max(0.0));
            let available = (width - margin[1] - margin[3] - padding[1] - padding[3]).max(1.0);
            content_width = style.width.resolve(ctx).unwrap_or(available);
            if let Some(max) = style.max_width.resolve(ctx) {
                content_width = content_width.min(max.max(1.0));
            }
            if let Some(min) = style.min_width.resolve(ctx) {
                content_width = content_width.max(min.max(1.0));
            }
            content_width = content_width.clamp(1.0, available);
            let auto_left = matches!(style.margin[3], css::values::SizeValue::Auto);
            let auto_right = matches!(style.margin[1], css::values::SizeValue::Auto);
            let extra = (available - content_width).max(0.0);
            self.page_origin = (
                margin[3]
                    + padding[3]
                    + if auto_left && auto_right {
                        extra / 2.0
                    } else if auto_left {
                        extra
                    } else {
                        0.0
                    },
                margin[0] + padding[0],
            );
        }
        let mut y = 10.0;
        let mut blocks = Vec::with_capacity(self.article.blocks.len());
        for (index, block) in self.article.blocks.iter().enumerate() {
            let view = if let Some(style) = self
                .article
                .block_styles
                .get(index)
                .and_then(|s| s.as_ref())
            {
                let spans = self
                    .article
                    .inline_styles
                    .get(index)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                BlockView::layout_styled(
                    &mut self.font_system,
                    block,
                    content_width,
                    y,
                    style.clone(),
                    spans,
                    env,
                )
            } else {
                BlockView::layout(&mut self.font_system, block, content_width, y)
            };
            y += view.height;
            blocks.push(view);
        }
        self.blocks = blocks;
        self.content_h = y + self.page_origin.1 + 10.0;
    }

    fn show_welcome(&mut self) {
        self.previous_presentation = None;
        self.previous_navigation = None;
        self.local_page = None;
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::Stop {
                id: self.navigation_id,
            });
        }
        self.navigation_id = self.next_navigation();
        self.load = LoadState::Idle;
        self.set_article(welcome_article(), View::Welcome);
        self.omnibox.set_text("");
        self.script_ready = false;
        self.navigation_url = None;
        if let Ok(blank) = Url::parse("about:blank") {
            if self.history.current() != Some(&blank) {
                self.history.navigate(blank);
            }
        }
        self.save_session();
        self.update_title();
        self.request_redraw();
    }

    fn on_fetch_event(&mut self, event: FetchEvent) {
        let id = match &event {
            FetchEvent::Committed { id, .. } => *id,
            FetchEvent::Favicon { id, .. } => *id,
            FetchEvent::Download { id, .. } => *id,
            FetchEvent::Crashed { id, .. } => *id,
            FetchEvent::Started { id, .. }
            | FetchEvent::WebRequest { id, .. }
            | FetchEvent::Dialog { id, .. }
            | FetchEvent::SameDocument { id, .. }
            | FetchEvent::WebNavigate { id, .. }
            | FetchEvent::ScriptPage { id, .. }
            | FetchEvent::ScriptError { id, .. }
            | FetchEvent::Navigate { id, .. }
            | FetchEvent::Progress { id, .. }
            | FetchEvent::Preview { id, .. }
            | FetchEvent::Done { id, .. }
            | FetchEvent::Fonts { id, .. }
            | FetchEvent::Image { id, .. }
            | FetchEvent::Styled { id, .. }
            | FetchEvent::Stopped { id } => *id,
        };
        if id != self.navigation_id && !matches!(event, FetchEvent::Crashed { .. }) {
            return;
        }
        match event {
            FetchEvent::Committed { url, .. } => {
                self.commit_history(&url);
                self.committed = true;
                self.previous_navigation = None;
                self.previous_presentation = None;
                self.set_article(
                    Article {
                        title: Some("Loading…".into()),
                        ..Default::default()
                    },
                    View::Welcome,
                );
                self.local_page = None;
                self.navigation_url = Some(url.clone());
                if !self.omnibox_focused {
                    self.omnibox.set_text(url.as_str());
                }
                self.save_session();
            }
            FetchEvent::Favicon { image, .. } => self.favicon = Some(image),
            FetchEvent::Download { source, .. } => {
                self.restore_previous_navigation();
                self.offer_download(source);
            }
            FetchEvent::Crashed { message, .. } => {
                if let Some(fetcher) = &self.fetcher {
                    fetcher.send(Command::Stop {
                        id: self.navigation_id,
                    });
                }
                self.previous_presentation = None;
                self.previous_navigation = None;
                self.crashed = true;
                self.load = LoadState::Idle;
                self.script_ready = false;
                self.set_article(
                    Article {
                        title: Some("Tab crashed".into()),
                        blocks: vec![
                            Block::Heading {
                                level: 1,
                                text: "This tab stopped".into(),
                                links: Vec::new(),
                            },
                            Block::Paragraph {
                                text: message,
                                links: Vec::new(),
                            },
                        ],
                        ..Default::default()
                    },
                    View::Error,
                );
            }
            FetchEvent::Dialog { request, reply, .. } => {
                self.queue_web_dialog(crate::web_ui::WebDialog::native(request, reply))
            }
            FetchEvent::WebRequest {
                origin, command, ..
            } => self.web_request(origin, command),
            FetchEvent::SameDocument {
                url,
                back,
                forward,
                replace,
                traversal,
                ..
            } => {
                self.spa_back = back;
                self.spa_forward = forward;
                self.omnibox.set_text(url.as_str());
                if let Some(delta) = traversal {
                    for _ in 0..delta.unsigned_abs().min(100) {
                        if delta < 0 {
                            self.history.go_back();
                        } else {
                            self.history.go_forward();
                        }
                    }
                    self.history.replace(url.clone());
                } else if replace {
                    self.history.replace(url.clone());
                } else {
                    self.history.navigate(url.clone());
                }
                self.navigation_url = Some(url.clone());
                self.profile
                    .data
                    .visit(&url, self.article.title.as_deref().unwrap_or(""));
                self.save_session();
                self.update_title();
            }
            FetchEvent::WebNavigate { url, replace, .. } => {
                self.omnibox.set_text(url.as_str());
                self.start_fetch(url, false, !replace);
            }
            FetchEvent::ScriptPage {
                page,
                styles,
                focused,
                console,
                ..
            } => {
                self.page = Some(page);
                self.styles = styles;
                self.dom_focused = focused;
                self.script_console = console;
                self.refresh_styled_article();
            }
            FetchEvent::ScriptError {
                message, stopped, ..
            } => {
                self.script_console.push(message.clone());
                if stopped {
                    self.script_dialog = Some(message);
                    self.script_ready = false;
                }
            }
            FetchEvent::Navigate { url, .. } => {
                self.omnibox.set_text(url.as_str());
                self.start_fetch(url, false, true);
            }
            FetchEvent::Started { url, .. } => {
                self.load = LoadState::Loading {
                    url,
                    downloaded: 0,
                    total: None,
                };
                self.update_title();
            }
            FetchEvent::Progress {
                downloaded, total, ..
            } => {
                if let LoadState::Loading {
                    downloaded: d,
                    total: t,
                    ..
                } = &mut self.load
                {
                    *d = downloaded;
                    *t = total;
                }
            }
            FetchEvent::Done { url, result, .. } => {
                self.previous_navigation = None;
                self.previous_presentation = None;
                self.script_ready = result.is_ok();
                self.load = LoadState::Idle;
                match result {
                    Ok((page, styles)) => self.show_document(&url, page, styles),
                    Err(err) => self.show_error(&url, &err),
                }
            }
            FetchEvent::Preview { page, styles, .. } => {
                if !self.preview_seen {
                    self.clear_web_fonts();
                    self.scroll_offsets.clear();
                    self.scroll_y = 0.0;
                    self.scroll_x = 0.0;
                    self.preview_seen = true;
                }
                self.page = Some(page);
                self.styles = styles;
                self.refresh_styled_article();
            }
            FetchEvent::Stopped { .. } => {
                if !self.crashed && !self.committed {
                    self.restore_previous_navigation();
                }
                if self.committed {
                    self.script_ready = false;
                }
                self.load = LoadState::Idle;
                self.update_title();
            }
            FetchEvent::Styled {
                revision,
                page,
                styles,
                ..
            } => {
                if revision == self.style_revision
                    && self
                        .page
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &page))
                {
                    self.styles = styles;
                    self.refresh_styled_article();
                }
            }
            FetchEvent::Fonts { page, assets, .. } => {
                if self
                    .page
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &page))
                {
                    let added = crate::fonts::install_fonts_db(&mut self.page_font_db, &assets);
                    if !added.is_empty() {
                        self.web_font_ids.extend(added);
                        self.font_generation = self.font_generation.wrapping_add(1);
                        self.content_version = self.content_version.wrapping_add(1);
                    }
                }
            }
            FetchEvent::Image { page, .. } => {
                if self
                    .page
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &page))
                {
                    self.content_version = self.content_version.wrapping_add(1);
                }
            }
        }
        self.request_redraw();
    }

    fn restore_previous_navigation(&mut self) {
        if let Some(presentation) = self.previous_presentation.take() {
            presentation.restore(self);
        }
        if let Some((id, ready)) = self.previous_navigation.take() {
            self.navigation_id = id;
            self.script_ready = ready;
        }
        self.load = LoadState::Idle;
        self.pending_traversal = 0;
        self.navigation_url = self.history.current().cloned();
        if let Some(url) = self.history.current() {
            self.omnibox.set_text(if url.as_str() == "about:blank" {
                ""
            } else {
                url.as_str()
            });
        }
        self.update_title();
        if self.script_ready {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::DomEvent {
                    id: self.navigation_id,
                    event: crate::script_worker::DomCommand::Snapshot,
                });
            }
        }
    }
    fn commit_history(&mut self, url: &Url) {
        if self.pending_traversal < 0 {
            self.history.go_back();
        } else if self.pending_traversal > 0 {
            self.history.go_forward();
        }
        if self.pending_history {
            self.history.navigate(url.clone());
        } else {
            self.history.replace(url.clone());
        }
        self.pending_history = false;
        self.pending_traversal = 0;
    }

    fn queue_web_dialog(&mut self, dialog: crate::web_ui::WebDialog) {
        if self.web_dialog.is_none() {
            self.web_dialog = Some(dialog);
        } else if self.web_dialogs.len() < 4 {
            self.web_dialogs.push_back(dialog);
        } else {
            self.resolve_web_dialog(dialog, 0);
        }
        self.request_redraw();
    }
    fn web_request(&mut self, origin: String, command: web_api::WebCommand) {
        use crate::web_ui::{Action, Kind, WebDialog};
        match command {
            web_api::WebCommand::Download {
                url,
                filename,
                bytes,
                gesture,
            } => {
                let top = self
                    .page
                    .as_ref()
                    .map_or_else(|| url.clone(), |p| p.url.clone());
                let filename = if filename.is_empty() {
                    crate::downloads::suggested_filename(&url, None)
                } else {
                    filename
                };
                self.offer_download(crate::downloads::Source {
                    force_prompt: !gesture || (top.scheme() == "https" && url.scheme() == "http"),
                    url,
                    top,
                    filename,
                    bytes,
                });
            }
            web_api::WebCommand::Permission { id, name, .. } => {
                let mut dialog=WebDialog::notice(format!("This page requests {name}. Choose once, always, or never. Saved grants can be revoked at about:permissions."));
                dialog.origin = origin;
                dialog.kind = Kind::Permission {
                    request: id,
                    name,
                    action: Action::Permission,
                };
                self.queue_web_dialog(dialog);
            }
            web_api::WebCommand::Clipboard {
                id,
                text,
                gesture,
                permission,
            } => {
                let action = Action::Clipboard(text);
                if !gesture || permission.as_deref() == Some("denied") {
                    self.web_decision(id, "clipboard-read".into(), "denied".into(), action);
                } else if permission.as_deref() == Some("granted") {
                    self.web_decision(id, "clipboard-read".into(), "granted".into(), action);
                } else {
                    let mut dialog=WebDialog::notice("This page requests clipboard text access. The desktop clipboard is read or changed only after approval.");
                    dialog.origin = origin;
                    dialog.kind = Kind::Permission {
                        request: id,
                        name: "clipboard-read".into(),
                        action,
                    };
                    self.queue_web_dialog(dialog);
                }
            }
            web_api::WebCommand::Popup { url, gesture } => {
                let mut dialog = WebDialog::notice(if gesture {
                    format!("Open a separate, isolated tab for {url}?")
                } else {
                    "Popup blocked: no user gesture.".into()
                });
                dialog.origin = origin;
                if gesture {
                    dialog.kind = Kind::Popup(url);
                }
                self.queue_web_dialog(dialog);
            }
            web_api::WebCommand::Notice(message) => {
                self.queue_web_dialog(WebDialog::notice(message))
            }
            web_api::WebCommand::Fullscreen { enter } => {
                if self.handling_background {
                    return;
                }
                if let Some(window) = &self.window {
                    window.set_fullscreen(if enter {
                        Some(winit::window::Fullscreen::Borderless(None))
                    } else {
                        None
                    });
                }
            }
            _ => {}
        }
    }
    fn web_decision(
        &self,
        request: u64,
        name: String,
        decision: String,
        action: crate::web_ui::Action,
    ) {
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::WebDecision {
                id: self.navigation_id,
                request,
                name,
                decision,
                action,
            });
        }
    }
    fn choose_web_dialog(&mut self, choice: usize) {
        if let Some(dialog) = self.web_dialog.take() {
            self.resolve_web_dialog(dialog, choice);
        }
        self.web_dialog = self.web_dialogs.pop_front();
        self.request_redraw();
    }
    fn resolve_web_dialog(&mut self, dialog: crate::web_ui::WebDialog, choice: usize) {
        use crate::web_ui::Kind;
        match dialog.kind {
            Kind::Permission {
                request,
                name,
                action,
            } => self.web_decision(
                request,
                name,
                match choice {
                    1 => "once",
                    2 => "always",
                    3 => "never",
                    _ => "denied",
                }
                .into(),
                action,
            ),
            Kind::Dialog { kind, reply } => {
                let value = if choice > 0 || matches!(kind, web_api::runtime::DialogKind::Alert) {
                    Some(dialog.input.text().to_owned())
                } else {
                    None
                };
                let _ = reply.send(value);
            }
            Kind::Popup(url) => {
                if choice == 1 {
                    if let Err(error) = self.new_tab(Some(url.as_str())) {
                        self.browser.notice = error;
                    }
                }
            }
            Kind::Settings => {
                if matches!(choice, 1 | 2) {
                    if let Some(fetcher) = &self.fetcher {
                        fetcher.send(Command::RevokePermissions {
                            id: self.navigation_id,
                            all: choice == 2,
                        });
                    }
                }
            }
            Kind::Notice => {}
        }
    }
    fn web_dialog_key(&mut self, event: &KeyEvent) {
        let prompt = self.web_dialog.as_ref().is_some_and(|d| {
            matches!(
                d.kind,
                crate::web_ui::Kind::Dialog {
                    kind: web_api::runtime::DialogKind::Prompt,
                    ..
                }
            )
        });
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.choose_web_dialog(0),
            Key::Named(NamedKey::Enter) => self.choose_web_dialog(1),
            Key::Character(key) if !prompt => {
                if let Ok(n) = key.parse::<usize>() {
                    if (1..=3).contains(&n) {
                        self.choose_web_dialog(n);
                    }
                }
            }
            _ if prompt => {
                if let Some(dialog) = &mut self.web_dialog {
                    match &event.logical_key {
                        Key::Named(NamedKey::Backspace) => dialog.input.backspace(),
                        Key::Named(NamedKey::Delete) => dialog.input.delete(),
                        Key::Named(NamedKey::ArrowLeft) => dialog.input.move_caret(-1),
                        Key::Named(NamedKey::ArrowRight) => dialog.input.move_caret(1),
                        _ => {
                            if let Some(text) = &event.text {
                                if !text.chars().any(char::is_control)
                                    && dialog.input.text().len() + text.len() <= 4096
                                {
                                    dialog.input.insert(text);
                                }
                            }
                        }
                    }
                }
                self.request_redraw();
            }
            _ => {}
        }
    }
    fn show_document(&mut self, requested: &Url, page: Arc<Page>, styles: ComputedStyles) {
        self.favicon = None;
        if !self.committed {
            self.commit_history(&page.url);
        }
        self.pending_history = false;
        self.clear_web_fonts();
        if !self.preview_seen {
            self.scroll_y = 0.0;
            self.scroll_x = 0.0;
            self.scroll_offsets.clear();
        }
        self.redirected_from = (requested != &page.url).then(|| requested.clone());
        self.omnibox.set_text(page.url.as_str());
        self.navigation_url = Some(page.url.clone());
        self.zoom = self
            .profile
            .data
            .zoom
            .get(&page.url.origin().ascii_serialization())
            .copied()
            .unwrap_or(1.0);
        self.profile
            .data
            .visit(&page.url, page.document.title().as_deref().unwrap_or(""));
        self.page = Some(page);
        self.styles = styles;
        self.refresh_styled_article();
        self.queue_restyle();
        self.update_title();
        self.save_session();
    }

    fn environment(&self) -> Environment {
        Environment {
            width: self.size.width.saturating_sub(SCROLLBAR_W as u32) as f32,
            height: self.viewport_h(),
            dark: self.dark_theme,
        }
    }

    fn queue_restyle(&mut self) {
        if let (Some(fetcher), Some(page)) = (&self.fetcher, &self.page) {
            self.style_revision = self.style_revision.wrapping_add(1);
            fetcher.send(Command::Restyle {
                id: self.navigation_id,
                revision: self.style_revision,
                page: page.clone(),
                environment: self.environment(),
            });
        }
    }

    fn refresh_styled_article(&mut self) {
        let Some(page) = &self.page else {
            return;
        };
        let body_style = page
            .document
            .get_elements_by_tag_name("body")
            .first()
            .and_then(|&id| self.styles.get(id))
            .cloned();
        let article = Article {
            title: page
                .document
                .title()
                .map(|title| profile::short_title(&title)),
            page_style: body_style,
            ..Article::default()
        };
        let root_bg = page
            .document
            .document_element()
            .and_then(|id| self.styles.get(id))
            .map(|s| s.background_color)
            .unwrap_or(css::values::Color::TRANSPARENT);
        self.canvas_color = if root_bg.a > 0 {
            root_bg
        } else {
            article
                .page_style
                .as_ref()
                .map(|s| s.background_color)
                .unwrap_or(css::values::Color::TRANSPARENT)
        };
        let scroll = self.scroll_y;
        self.set_article(article, View::Document);
        self.scroll_y = scroll;
    }

    fn clear_web_fonts(&mut self) {
        if self.web_font_ids.is_empty() {
            return;
        }
        self.web_font_ids.clear();
        self.page_font_db = self.font_system.db().clone();
        self.font_generation = self.font_generation.wrapping_add(1);
    }

    fn show_error(&mut self, url: &Url, err: &NetError) {
        self.commit_history(url);
        self.omnibox.set_text(url.as_str());
        self.pending_history = false;
        let (title, advice) = match err {
            NetError::Tls(tls) => (
                format!("[TLS ERROR — {tls}]"),
                "The connection is not private: the certificate cannot be trusted.\nGo Back (Alt+Left) or correct the certificate before reloading. Certificate verification was not bypassed.",
            ),
            NetError::HttpStatus(status) => (
                format!("[HTTP {status}]"),
                "The server answered with an error status. The page was not rendered.",
            ),
            _ => (
                "[NETWORK ERROR]".to_owned(),
                "Check your connection and the address, then Reload (Ctrl+R) or go Back (Alt+Left).",
            ),
        };
        let article = Article {
            title: Some(title.clone()),
            blocks: vec![
                Block::Heading {
                    level: 1,
                    text: title,
                    links: Vec::new(),
                },
                Block::Paragraph {
                    text: format!("URL: {}\n\n{err}\n\n{advice}", url.as_str()),
                    links: Vec::new(),
                },
            ],
            ..Article::default()
        };
        self.set_article(article, View::Error);
        self.update_title();
        self.save_session();
    }

    fn update_title(&mut self) {
        if self.handling_background {
            return;
        }
        let Some(window) = &self.window else { return };
        let title = match &self.load {
            LoadState::Loading { url, .. } => {
                format!("browser — {} (loading…)", host_or_url(url))
            }
            LoadState::Idle => {
                let host = self
                    .history
                    .current()
                    .map(host_or_url)
                    .unwrap_or_else(|| "browser".to_owned());
                match (&self.view, &self.article.title) {
                    (View::Document, Some(doc_title)) if !doc_title.is_empty() => {
                        format!("{doc_title} — {host}")
                    }
                    _ => format!("browser — {host}"),
                }
            }
        };
        if self.last_window_title != title {
            self.last_window_title = title.clone();
            window.set_title(&title);
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn drain_fetcher(&mut self) {
        // Collect first to end the fetcher borrow before mutating self.
        let original = self.active_tab;
        self.batch_visible = Some(original);
        let mut batches = Vec::new();
        let mut events = Vec::new();
        if let Some(fetcher) = &self.fetcher {
            fetcher.drain_events(|event| events.push(event));
        }
        batches.push((original, events));
        for (index, tab) in self.tabs.iter().enumerate() {
            if let Some(fetcher) = tab.as_ref().and_then(|t| t.fetcher.as_ref()) {
                let mut events = Vec::new();
                fetcher.drain_events(|event| events.push(event));
                batches.push((index, events));
            }
        }
        for (index, events) in batches {
            if events.is_empty() {
                continue;
            }
            self.handling_background = index != original;
            self.swap_context(index);
            for event in events {
                self.on_fetch_event(event);
            }
        }
        self.swap_context(original);
        self.handling_background = false;
        self.batch_visible = None;
        if self.session_dirty {
            self.session_dirty = false;
            self.save_session();
        }
        self.update_title();
    }

    fn drain_layout(&mut self) {
        let ready = self
            .layout_worker
            .as_ref()
            .map_or_else(Vec::new, |w| w.drain());
        for result in ready {
            if result.navigation != self.navigation_id
                || result.revision != self.content_version
                || result.viewport.width != self.environment().width.max(1.0)
                || result.viewport.height != self.viewport_h()
            {
                continue;
            }
            if let Some(message) = result.error {
                self.on_fetch_event(FetchEvent::Crashed {
                    id: self.navigation_id,
                    message,
                });
                continue;
            }
            self.content_h = result.layout.content_size.height * self.zoom;
            if let Some((x, y)) = self.restore_scroll.take() {
                self.scroll_x = x;
                self.scroll_y = y;
            }
            self.flow = Some(Arc::new(result.layout));
            self.browser.find_key = None;
            self.scene = result.scene;
            self.flow_navigation = Some(result.navigation);
            self.layout_ms = result.elapsed_ms;
            self.layout_passes = result.passes;
            self.blocks.clear();
            self.page_origin = (0.0, 0.0);
            self.send_geometry();
            self.request_redraw();
        }
    }

    // ---------- input ----------

    fn on_key(&mut self, event: &KeyEvent) {
        if event.state == ElementState::Released {
            if !self.omnibox_focused
                && !self.browser.find_focused
                && self.browser.editor.is_none()
                && self.browser.focused.is_none()
                && self.script_ready
            {
                self.send_dom_key(event, false);
            }
            return;
        }
        if self.handle_browser_key(event) {
            return;
        }
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();

        // Global shortcuts first.
        if ctrl && !alt {
            match &event.logical_key {
                Key::Character(ch) if ch.as_str() == "l" || ch.as_str() == "L" => {
                    self.focus_omnibox();
                    return;
                }
                Key::Character(ch) if ch.as_str() == "r" || ch.as_str() == "R" => {
                    if !event.repeat {
                        self.reload();
                    }
                    return;
                }
                _ => {}
            }
        }
        if alt && !ctrl {
            match &event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => {
                    if !event.repeat {
                        self.go_back();
                    }
                    return;
                }
                Key::Named(NamedKey::ArrowRight) => {
                    if !event.repeat {
                        self.go_forward();
                    }
                    return;
                }
                _ => {}
            }
        }

        if self.web_dialog.is_some() {
            self.web_dialog_key(event);
            return;
        }
        if matches!(event.logical_key, Key::Named(NamedKey::Escape))
            && self
                .window
                .as_ref()
                .is_some_and(|w| w.fullscreen().is_some())
        {
            if let Some(window) = &self.window {
                window.set_fullscreen(None);
            }
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::DomEvent {
                    id: self.navigation_id,
                    event: crate::script_worker::DomCommand::ExitFullscreen,
                });
            }
            return;
        }

        if self.omnibox_focused {
            self.on_key_omnibox(event);
        } else {
            self.on_key_viewport(event);
        }
    }

    /// Focus the address bar with its content selected.
    fn focus_omnibox(&mut self) {
        self.browser.find_focused = false;
        self.browser.focused = None;
        self.browser.suggestion = None;
        self.omnibox_focused = true;
        self.omnibox.select_all();
        self.request_redraw();
    }

    fn on_key_omnibox(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                if !event.repeat {
                    self.submit_omnibox();
                }
            }
            Key::Named(NamedKey::Escape) => {
                self.omnibox_focused = false;
                self.request_redraw();
            }
            Key::Named(NamedKey::Backspace) => {
                self.omnibox.backspace();
                self.request_redraw();
            }
            Key::Named(NamedKey::Delete) => {
                self.omnibox.delete();
                self.request_redraw();
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.omnibox.move_caret(-1);
                self.request_redraw();
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let count = self.profile.data.suggestions(self.omnibox.text(), 6).len();
                if count > 0 {
                    let current = self.browser.suggestion.unwrap_or(count - 1);
                    self.browser.suggestion =
                        Some(if event.logical_key == Key::Named(NamedKey::ArrowUp) {
                            (current + count - 1) % count
                        } else {
                            (current + 1) % count
                        });
                }
                self.request_redraw();
            }
            Key::Named(NamedKey::Tab) => {
                self.omnibox_focused = false;
                self.browser.focused = Some(0);
                self.request_redraw();
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.omnibox.move_caret(1);
                self.request_redraw();
            }
            Key::Named(NamedKey::Home) => {
                self.omnibox.jump_to(false);
                self.request_redraw();
            }
            Key::Named(NamedKey::End) => {
                self.omnibox.jump_to(true);
                self.request_redraw();
            }
            _ => {
                if let Some(text) = event.text.as_ref() {
                    if !text.chars().any(|c| c.is_control()) {
                        if self.omnibox.is_selected()
                            || self.omnibox.text().len() + text.len() <= 8192
                        {
                            self.omnibox.insert(text.as_str());
                            self.browser.suggestion = None;
                        }
                        self.request_redraw();
                    }
                }
            }
        }
    }

    fn on_key_viewport(&mut self, event: &KeyEvent) {
        if self.script_dialog.is_some()
            && matches!(
                event.logical_key,
                Key::Named(NamedKey::Enter | NamedKey::Escape)
            )
        {
            self.script_dialog = None;
            self.request_redraw();
            return;
        }
        if self.script_ready {
            if matches!(event.logical_key, Key::Named(NamedKey::Tab)) {
                if let Some(fetcher) = &self.fetcher {
                    fetcher.send(Command::DomEvent {
                        id: self.navigation_id,
                        event: crate::script_worker::DomCommand::Tab {
                            reverse: self.modifiers.shift_key(),
                        },
                    });
                }
                return;
            }
            if self.dom_focused.is_some()
                || matches!(
                    event.logical_key,
                    Key::Character(_)
                        | Key::Named(NamedKey::Backspace | NamedKey::Delete | NamedKey::Enter)
                )
            {
                self.send_dom_key(event, true);
                return;
            }
        }
        match &event.logical_key {
            Key::Named(NamedKey::PageDown) => {
                self.scroll_by(self.viewport_h() * 0.9);
            }
            Key::Named(NamedKey::PageUp) => {
                self.scroll_by(-self.viewport_h() * 0.9);
            }
            Key::Named(NamedKey::Home) => {
                self.scroll_y = 0.0;
                self.request_redraw();
            }
            Key::Named(NamedKey::End) => {
                self.scroll_y = f32::MAX;
                self.request_redraw();
            }
            Key::Named(NamedKey::Escape) => {
                self.stop();
            }
            _ => {}
        }
    }
    fn send_dom_key(&self, event: &KeyEvent, pressed: bool) {
        if let Some(fetcher) = &self.fetcher {
            let key = match &event.logical_key {
                Key::Character(s) => s.to_string(),
                Key::Named(k) => format!("{k:?}"),
                _ => String::new(),
            };
            fetcher.send(Command::DomEvent {
                id: self.navigation_id,
                event: crate::script_worker::DomCommand::Key {
                    node: None,
                    key,
                    pressed,
                    text: if pressed && !self.modifiers.control_key() && !self.modifiers.alt_key() {
                        event
                            .text
                            .as_ref()
                            .map(|s| s.to_string())
                            .filter(|s| !s.chars().any(char::is_control))
                    } else {
                        None
                    },
                },
            });
        }
    }

    fn on_wheel(&mut self, delta: &MouseScrollDelta) {
        let (mut dx, mut dy) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (-*x * LINE_SCROLL_PX, -*y * LINE_SCROLL_PX),
            MouseScrollDelta::PixelDelta(pos) => (-(pos.x as f32), -(pos.y as f32)),
        };
        if self.modifiers.control_key() {
            self.zoom_by((-dy / 600.0).exp());
            return;
        }
        if self.modifiers.shift_key() {
            std::mem::swap(&mut dx, &mut dy);
        }
        if let Some(flow) = &self.flow {
            let x = (self.mouse_x as f32 + self.scroll_x) / self.zoom;
            let y = (self.mouse_y as f32 - TOOLBAR_H as f32 + self.scroll_y) / self.zoom;
            if flow.scroll_by_at(
                x,
                y,
                (dx / self.zoom, dy / self.zoom),
                &mut self.scroll_offsets,
            ) {
                self.send_geometry();
                self.request_redraw();
                return;
            }
            let max = (flow.content_size.width * self.zoom - self.environment().width).max(0.0);
            self.scroll_x = (self.scroll_x + dx).clamp(0.0, max);
        }
        self.scroll_by(dy);
    }

    fn zoom_by(&mut self, factor: f32) {
        let old = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.8, 2.0);
        if let Some(flow) = &self.flow {
            self.content_h = flow.content_size.height * self.zoom;
        }
        self.scroll_y *= self.zoom / old;
        self.scroll_x *= self.zoom / old;
        if let Some(page) = &self.page {
            self.profile
                .data
                .zoom
                .insert(page.url.origin().ascii_serialization(), self.zoom);
            self.profile.changed();
        }
        self.send_geometry();
        self.request_redraw();
    }

    fn scroll_by(&mut self, dy: f32) {
        let max = (self.content_h - self.viewport_h()).max(0.0);
        self.scroll_y = (self.scroll_y + dy).clamp(0.0, max);
        self.request_redraw();
    }

    fn viewport_h(&self) -> f32 {
        (self.size.height as i32
            - TOOLBAR_H
            - STATUS_H
            - if self.browser.find_open { FIND_H } else { 0 })
        .max(50) as f32
    }
    fn web_viewport(&self) -> web_api::runtime::GeometryRect {
        web_api::runtime::GeometryRect {
            x: (self.scroll_x / self.zoom) as f64,
            y: (self.scroll_y / self.zoom) as f64,
            width: (self.environment().width / self.zoom) as f64,
            height: (self.viewport_h() / self.zoom) as f64,
        }
    }
    fn send_geometry(&self) {
        if !self.script_ready {
            return;
        }
        if let (Some(fetcher), Some(flow)) = (&self.fetcher, &self.flow) {
            fetcher.send(Command::DomEvent {
                id: self.navigation_id,
                event: crate::script_worker::DomCommand::Geometry {
                    layout: flow.clone(),
                    offsets: self.scroll_offsets.clone(),
                    viewport: self.web_viewport(),
                },
            });
        }
    }

    fn on_click(&mut self) {
        if self.web_dialog.is_some() {
            let (x, y, w, h) = crate::web_ui::WebDialog::rect(self.size.width);
            if self.mouse_x >= x as f64
                && self.mouse_x < (x + w) as f64
                && self.mouse_y >= (y + h - 44) as f64
                && self.mouse_y < (y + h) as f64
            {
                let choice = (((self.mouse_x - x as f64) / (w as f64 / 3.0)) as usize + 1).min(3);
                self.choose_web_dialog(choice);
            }
            return;
        }
        if self.script_dialog.take().is_some() {
            self.request_redraw();
            return;
        }
        if let Some(action) = self
            .browser
            .widgets
            .iter()
            .rev()
            .find(|widget| {
                let (x, y, w, h) = widget.rect;
                widget.action.is_some()
                    && self.mouse_x >= x as f64
                    && self.mouse_x < (x + w) as f64
                    && self.mouse_y >= y as f64
                    && self.mouse_y < (y + h) as f64
            })
            .and_then(|w| w.action.clone())
        {
            if let ChromeAction::Select(index) = action {
                self.browser.dragged_tab = Some(index);
                self.chrome_action(ChromeAction::Select(index));
            } else {
                self.chrome_action(action);
            }
            return;
        }
        if self.browser.editor.is_some() {
            return;
        }
        if self.browser.menu {
            self.browser.menu = false;
            self.request_redraw();
            return;
        }
        self.browser.focused = None;
        self.browser.find_focused = false;
        let (mx, my) = (self.mouse_x as i32, self.mouse_y as i32);
        // Toolbar buttons.
        let layout = self.toolbar_layout();
        for (index, button) in layout.buttons.iter().enumerate() {
            if !button.enabled {
                continue;
            }
            let (x, y, w, h) = button.rect;
            if mx >= x && mx < x + w && my >= y && my < y + h {
                match index {
                    0 => self.go_back(),
                    1 => self.go_forward(),
                    2 => self.stop_or_reload(),
                    3 => self.chrome_action(ChromeAction::Home),
                    4 => self.chrome_action(ChromeAction::Security),
                    5 => self.chrome_action(ChromeAction::Bookmark),
                    6 => self.chrome_action(ChromeAction::Local(LocalPage::Downloads)),
                    _ => self.chrome_action(ChromeAction::Menu),
                }
                return;
            }
        }
        // Omnibox focus (selects all, like desktop browsers).
        let (x, y, w, h) = layout.omnibox_rect;
        if mx >= x && mx < x + w && my >= y && my < y + h {
            self.focus_omnibox();
            return;
        }
        // Scrollbar drag (viewport area).
        let (vx, vy, vw, vh) = self.viewport_rect();
        let track_x = vx + vw - SCROLLBAR_W;
        if mx >= track_x && mx < vx + vw && my >= vy && my < vy + vh {
            self.dragging_scroll = true;
            self.drag_scroll_to(my);
            return;
        }
        // Viewport text click: follow links.
        let (vx, vy, vw, vh) = self.viewport_rect();
        if mx >= vx && mx < vx + vw - SCROLLBAR_W && my >= vy && my < vy + vh {
            let content_y = my as f32 - vy as f32 + self.scroll_y - self.page_origin.1;
            if self.script_ready {
                if let Some(flow) = &self.flow {
                    if let Some(element) = flow.hit_element_scrolled(
                        (mx as f32 - vx as f32 + self.scroll_x) / self.zoom,
                        content_y / self.zoom,
                        (self.scroll_x / self.zoom, self.scroll_y / self.zoom),
                        &self.scroll_offsets,
                    ) {
                        let mut node = element.node;
                        if let Some(page) = &self.page {
                            if let Some(n) = page.document.try_get(node) {
                                if matches!(n.data, html::NodeData::Text(_)) {
                                    node = n.parent.unwrap_or(node);
                                }
                            }
                        }
                        if let Some(fetcher) = &self.fetcher {
                            fetcher.send(Command::DomEvent {
                                id: self.navigation_id,
                                event: crate::script_worker::DomCommand::Click(node),
                            });
                        }
                        self.omnibox_focused = false;
                        self.request_redraw();
                        return;
                    }
                }
            }
            if let Some(href) = self.hit_link(
                mx as f32 - vx as f32 - self.page_origin.0 + self.scroll_x,
                content_y,
            ) {
                self.omnibox.set_text(href.as_str());
                self.start_fetch(href, false, true);
                return;
            }
        }
        self.omnibox_focused = false;
        self.request_redraw();
    }

    /// Link under viewport-content point `(x, y)` (content coords).
    fn hit_link(&mut self, x: f32, y: f32) -> Option<Url> {
        if let Some(flow) = &self.flow {
            return flow.hit_link_scrolled(
                x / self.zoom,
                y / self.zoom,
                (self.scroll_x / self.zoom, self.scroll_y / self.zoom),
                &self.scroll_offsets,
            );
        }
        for block in &mut self.blocks {
            if y >= block.y && y < block.y + block.height {
                return block.hit_link(&mut self.font_system, x, y - block.y);
            }
        }
        None
    }

    fn drag_scroll_to(&mut self, mouse_y: i32) {
        let (_, vy, _, vh) = self.viewport_rect();
        let max = (self.content_h - vh as f32).max(0.0);
        if max <= 0.0 {
            return;
        }
        let ratio = ((mouse_y - vy) as f32 / vh as f32).clamp(0.0, 1.0);
        self.scroll_y = ratio * max;
        self.request_redraw();
    }

    // ---------- layout ----------

    fn toolbar_layout(&self) -> ToolbarLayout {
        let w = self.size.width as i32;
        let mut x = PAD;
        let mut buttons = Vec::new();
        let loading = matches!(self.load, LoadState::Loading { .. });
        for (label, enabled) in [
            ("<", self.history.can_go_back() || self.spa_back),
            (">", self.history.can_go_forward() || self.spa_forward),
            (if loading { "[]" } else { "R" }, true),
            ("H", true),
            (
                if self
                    .history
                    .current()
                    .is_some_and(|u| u.scheme() == "https")
                {
                    "L"
                } else {
                    "!"
                },
                true,
            ),
            ("*", true),
            ("D", true),
            ("=", true),
        ] {
            buttons.push(Button {
                label,
                rect: (x, TABS_H + 6, BUTTON_W, NAV_H - 12),
                enabled,
            });
            x += BUTTON_W + BUTTON_GAP;
        }
        let omnibox_rect = (x, TABS_H + 6, (w - x - PAD).max(80), NAV_H - 12);
        ToolbarLayout {
            buttons,
            omnibox_rect,
        }
    }

    fn viewport_rect(&self) -> (i32, i32, i32, i32) {
        let w = self.size.width as i32;
        let h = self.size.height as i32;
        (
            0,
            TOOLBAR_H,
            w,
            (h - TOOLBAR_H - STATUS_H - if self.browser.find_open { FIND_H } else { 0 }).max(50),
        )
    }

    // ---------- rendering ----------

    fn render(&mut self) {
        let frame_started = std::time::Instant::now();
        let Some(window) = self.window.clone() else {
            return;
        };
        // Always track the real window size: compositors (tiling, decorations)
        // may assign anything, and the startup guess is often wrong. A stale
        // size makes softbuffer fail and the window never commits a frame.
        self.size = window.inner_size();
        if self.gpu.is_none() && self.surface.is_none() {
            let context = match Context::new(window.clone()) {
                Ok(context) => context,
                Err(err) => {
                    self.fatal_init = Some(format!("softbuffer context failed: {err}"));
                    return;
                }
            };
            match Surface::new(&context, window.clone()) {
                Ok(surface) => {
                    self.context = Some(context);
                    self.surface = Some(surface);
                }
                Err(err) => {
                    self.fatal_init = Some(format!("softbuffer surface failed: {err}"));
                    return;
                }
            }
        }
        let (w, h) = (self.size.width, self.size.height);
        let (Some(ww), Some(hh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else {
            return;
        };
        // Phase A: mutate text buffers and compute layout (no surface borrow).
        let (vx, vy, vw, vh) = self.viewport_rect();
        self.measure_content(vw as f32);
        let max_scroll = (self.content_h - vh as f32).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        let max_x = self.flow.as_ref().map_or(0.0, |f| {
            (f.content_size.width * self.zoom - self.environment().width).max(0.0)
        });
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
        let viewport = self.web_viewport();
        if self.script_ready && self.observer_viewport != Some(viewport) {
            self.observer_viewport = Some(viewport);
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::DomEvent {
                    id: self.navigation_id,
                    event: crate::script_worker::DomCommand::Viewport(viewport),
                });
            }
        }
        let status = self.status_line();
        self.status_text.set_text(&mut self.font_system, &status);
        self.status_text
            .set_size(&mut self.font_system, w as f32, STATUS_H as f32);
        let layout = self.toolbar_layout();
        self.request_find();
        self.browser.widgets = self.chrome_widgets();
        let find_quads = self.find_quads();
        let omni_w = (layout.omnibox_rect.2 - 16).max(10) as f32;
        self.omnibox_text
            .set_text(&mut self.font_system, self.omnibox.text());
        self.omnibox_text
            .set_size(&mut self.font_system, omni_w, h as f32);

        // Chrome uses the established text widgets. Only its small toolbar,
        // status and scrollbar strips are uploaded when the GPU is active.
        // Page fills, blending and image/glyph scaling run in the WGSL path.
        self.pixels.resize(w as usize * h as usize, 0);
        let fs = &mut self.font_system;
        let cache = &mut self.text_renderer.cache;
        let mut canvas = Canvas {
            w,
            h,
            pixels: &mut self.pixels,
        };
        canvas.fill_rect(0, 0, w as i32, h as i32, BG);
        canvas.fill_rect(vx, vy, vw, vh, VIEW_BG);
        let bg = self.canvas_color;
        canvas.blend_rect(
            vx,
            vy,
            vw.max(0) as u32,
            vh.max(0) as u32,
            (bg.r, bg.g, bg.b, bg.a),
        );
        let page_clip = layout::Rect::new(
            vx as f32,
            vy as f32,
            (vw - SCROLLBAR_W).max(1) as f32,
            vh as f32,
        );
        if self.flow.is_some() {
            if self.gpu.is_none() {
                self.scene.paint_software(
                    canvas.pixels,
                    w,
                    h,
                    page_clip,
                    (vx as f32 - self.scroll_x, vy as f32 - self.scroll_y),
                    self.zoom,
                    &self.scroll_offsets,
                );
            }
        } else {
            for block in &mut self.blocks {
                if block.y + block.height < self.scroll_y || block.y > self.scroll_y + vh as f32 {
                    continue;
                }
                block.draw(
                    fs,
                    cache,
                    &mut canvas,
                    vx + self.page_origin.0 as i32,
                    vy + self.page_origin.1 as i32 - self.scroll_y as i32,
                );
            }
        }
        if self.gpu.is_none() && !find_quads.is_empty() {
            let highlights =
                paint::Scene::from_quads(find_quads.iter().map(|(q, _)| q.clone()).collect());
            highlights.paint_software(
                canvas.pixels,
                w,
                h,
                page_clip,
                (0.0, 0.0),
                1.0,
                &Default::default(),
            );
        }
        paint_scrollbar(&mut canvas, vx, vy, vw, vh, self.content_h, self.scroll_y);
        canvas.fill_rect(0, 0, w as i32, TOOLBAR_H, CHROME_BG);
        canvas.fill_rect(0, TOOLBAR_H - 1, w as i32, 1, BORDER);
        for button in &layout.buttons {
            paint_button(fs, cache, &mut canvas, button);
        }
        paint_omnibox(
            fs,
            cache,
            &mut canvas,
            &self.omnibox,
            &mut self.omnibox_text,
            &mut self.omnibox_prefix,
            self.omnibox_focused,
            layout.omnibox_rect,
        );
        canvas.fill_rect(0, h as i32 - STATUS_H, w as i32, 1, BORDER);
        canvas.fill_rect(
            0,
            h as i32 - STATUS_H + 1,
            w as i32,
            STATUS_H - 1,
            CHROME_BG,
        );
        self.status_text
            .draw(fs, cache, &mut canvas, PAD, h as i32 - STATUS_H + 5);
        if self.local_page.is_some() {
            canvas.fill_rect(
                vx,
                vy,
                vw,
                vh,
                if self.dark_theme { 0x142234 } else { VIEW_BG },
            );
        }
        if self.browser.find_open {
            canvas.fill_rect(0, h as i32 - STATUS_H - FIND_H, w as i32, FIND_H, CHROME_BG);
        }
        for widget in &self.browser.widgets {
            widget.draw(fs, cache, &mut canvas, self.dark_theme);
        }
        for widget in &self.browser.widgets {
            if let Some(ChromeAction::Select(index)) = &widget.action {
                let icon = if *index == self.active_tab {
                    self.favicon.as_ref()
                } else {
                    self.tabs
                        .get(*index)
                        .and_then(|t| t.as_ref())
                        .and_then(|t| t.favicon.as_ref())
                };
                if let Some(icon) = icon {
                    let (x, y, _, _) = widget.rect;
                    for yy in 0..16 {
                        for xx in 0..16 {
                            let i = ((yy * icon.height / 16 * icon.width + xx * icon.width / 16)
                                * 4) as usize;
                            let p = &icon.rgba[i..i + 4];
                            canvas.blend_rect(
                                x + 4 + xx as i32,
                                y + 8 + yy as i32,
                                1,
                                1,
                                (p[0], p[1], p[2], p[3]),
                            );
                        }
                    }
                }
            }
        }
        let dialog_rect = self
            .script_dialog
            .as_ref()
            .map(|_| (16, TOOLBAR_H + 20, (w as i32 - 32).max(40), 150));
        if let (Some(message), Some((x, y, dw, dh))) = (&self.script_dialog, dialog_rect) {
            canvas.fill_rect(x, y, dw, dh, 0xfff3cd);
            canvas.fill_rect(x, y, dw, 3, 0xb45309);
            let mut text = TextBuffer::with_metrics(fs, 16.0, cosmic_text::Wrap::Word);
            text.set_text(fs,&format!("Long-running script stopped\n{message}\n\nStop script — click this dialog or press Enter/Escape."));
            text.set_size(fs, (dw - 24).max(1) as f32, (dh - 16) as f32);
            text.draw(fs, cache, &mut canvas, x + 12, y + 10);
        }
        if let Some(dialog) = &self.web_dialog {
            dialog.draw(fs, cache, &mut canvas);
        }
        window.pre_present_notify();
        if let Some(gpu) = &mut self.gpu {
            let window_clip = layout::Rect::new(0.0, 0.0, w as f32, h as f32);
            let mut quads = vec![(paint::Quad::solid(page_clip, self.canvas_color), page_clip)];
            if self.flow.is_some() {
                quads.extend(self.scene.visible_quads(
                    page_clip,
                    (vx as f32 - self.scroll_x, vy as f32 - self.scroll_y),
                    self.zoom,
                    &self.scroll_offsets,
                ));
                quads.extend(find_quads);
            } else if let Some(q) = chrome_quad(&self.pixels, w, h, (vx, vy, vw - SCROLLBAR_W, vh))
            {
                quads.push((q, page_clip));
            }
            for rect in [
                (0, 0, w as i32, TOOLBAR_H),
                (0, h as i32 - STATUS_H, w as i32, STATUS_H),
                (vx + vw - SCROLLBAR_W, vy, SCROLLBAR_W, vh),
            ] {
                if let Some(q) = chrome_quad(&self.pixels, w, h, rect) {
                    quads.push((q, window_clip));
                }
            }
            if self.browser.find_open {
                if let Some(q) = chrome_quad(
                    &self.pixels,
                    w,
                    h,
                    (0, h as i32 - STATUS_H - FIND_H, w as i32, FIND_H),
                ) {
                    quads.push((q, window_clip));
                }
            }
            for widget in self.browser.widgets.iter().filter(|widget| widget.overlay) {
                if let Some(q) = chrome_quad(&self.pixels, w, h, widget.rect) {
                    quads.push((q, window_clip));
                }
            }
            if let Some(rect) = dialog_rect {
                if let Some(q) = chrome_quad(&self.pixels, w, h, rect) {
                    quads.push((q, window_clip));
                }
            }
            if self.web_dialog.is_some() {
                if let Some(q) = chrome_quad(&self.pixels, w, h, crate::web_ui::WebDialog::rect(w))
                {
                    quads.push((q, window_clip));
                }
            }
            gpu.resize(w, h);
            if let Err(err) = gpu.present(&quads) {
                eprintln!("browser: GPU present failed, switching to software: {err}");
                self.gpu = None;
                self.software_render = true;
                self.request_redraw();
                return;
            }
        } else {
            let Some(surface) = &mut self.surface else {
                return;
            };
            if let Err(err) = surface.resize(ww, hh) {
                eprintln!("browser: surface resize failed: {err}");
                return;
            }
            let mut buffer = match surface.buffer_mut() {
                Ok(buffer) => buffer,
                Err(err) => {
                    eprintln!("browser: pixel buffer failed: {err}");
                    return;
                }
            };
            buffer.copy_from_slice(&self.pixels);
            if let Err(err) = buffer.present() {
                eprintln!("browser: present failed: {err}");
                return;
            }
        }
        self.frames += 1;
        if self.perf {
            if !self.painted_navigation
                && self.view == View::Document
                && self.flow_navigation == Some(self.navigation_id)
                && (self.preview_seen || matches!(self.load, LoadState::Idle))
            {
                let elapsed = self
                    .navigation_started
                    .map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
                eprintln!(
                    "PERF first_paint_ms={elapsed:.3} before_eof={}",
                    matches!(self.load, LoadState::Loading { .. })
                );
                self.painted_navigation = true;
            }
            let stats = self.flow.as_ref().map(|f| &f.stats);
            eprintln!("PERF frame={} renderer={} layout_passes={} layout_ms={:.3} frame_ms={:.3} scroll={:.1} zoom={:.2} dirty_nodes={} reused_boxes={} reused_subtrees={}", self.frames, if self.gpu.is_some() { "gpu" } else { "software" }, self.layout_passes, self.layout_ms, frame_started.elapsed().as_secs_f64() * 1000.0, self.scroll_y, self.zoom,stats.map_or(0,|s|s.dirty_nodes),stats.map_or(0,|s|s.reused_boxes),stats.map_or(0,|s|s.reused_subtrees));
        }
    }

    /// Rebuild laid-out blocks only when content or width changed.
    fn measure_content(&mut self, vw: f32) {
        if self.local_page == Some(LocalPage::Preferences) {
            self.content_h = 600.0;
            return;
        }
        let key = (self.content_version, vw as u32);
        if self.content_measured_for == Some(key) {
            return;
        }
        self.rebuild_blocks((vw - SCROLLBAR_W as f32).max(1.0));
        self.content_measured_for = Some(key);
    }

    fn status_line(&self) -> String {
        if !self.browser.notice.is_empty() {
            return self.browser.notice.clone();
        }
        if let Some(page) = self.local_page {
            return format!("{} · local profile · no remote assets", page.path());
        }
        match &self.load {
            LoadState::Loading {
                url,
                downloaded,
                total,
            } => {
                let progress = match total {
                    Some(t) if *t > 0 => {
                        format!("{}%", downloaded * 100 / (*t as usize).max(1))
                    }
                    _ => format_bytes(*downloaded),
                };
                format!("Loading {} … {}", host_or_url(url), progress)
            }
            LoadState::Idle => match self.view {
                View::Welcome => "Ready — type a URL and press Enter".to_owned(),
                View::Document => {
                    if let Some(flow) = &self.flow {
                        return format!(
                            "Done — {} boxes · {} lines · {} CSS sheets · {:.0}% · {}",
                            flow.stats.boxes,
                            flow.stats.lines,
                            self.page.as_ref().map_or(0, |p| p.stylesheets.len()),
                            self.zoom * 100.0,
                            if self.gpu.is_some() {
                                "GPU"
                            } else {
                                "software"
                            }
                        );
                    }
                    let links = self
                        .article
                        .blocks
                        .iter()
                        .filter_map(|block| match block {
                            Block::Heading { links, .. }
                            | Block::Paragraph { links, .. }
                            | Block::ListItem { links, .. } => Some(links.len()),
                            _ => None,
                        })
                        .sum::<usize>();
                    format!(
                        "Done — {} blocks · {links} links · {} CSS sheets",
                        self.article.blocks.len(),
                        self.page.as_ref().map_or(0, |p| p.stylesheets.len()),
                    )
                }
                View::Error => "Failed — see the error above".to_owned(),
            },
        }
    }
}

struct ToolbarLayout {
    buttons: Vec<Button>,
    omnibox_rect: (i32, i32, i32, i32),
}

fn chrome_quad(
    pixels: &[u32],
    width: u32,
    height: u32,
    rect: (i32, i32, i32, i32),
) -> Option<paint::Quad> {
    let (x, y, w, h) = rect;
    let x0 = x.max(0).min(width as i32) as u32;
    let y0 = y.max(0).min(height as i32) as u32;
    let x1 = x.saturating_add(w).max(0).min(width as i32) as u32;
    let y1 = y.saturating_add(h).max(0).min(height as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut rgba = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for row in y0..y1 {
        for col in x0..x1 {
            let p = *pixels.get((row * width + col) as usize)?;
            rgba.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]);
        }
    }
    let image = layout::RasterImage::new(x1 - x0, y1 - y0, rgba)?;
    Some(paint::Quad::image(
        layout::Rect::new(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32),
        image,
    ))
}

fn paint_button(
    font_system: &mut cosmic_text::FontSystem,
    cache: &mut cosmic_text::SwashCache,
    canvas: &mut Canvas<'_>,
    button: &Button,
) {
    let (x, y, w, h) = button.rect;
    let bg = if button.enabled { VIEW_BG } else { CHROME_BG };
    canvas.fill_rect(x, y, w, h, bg);
    canvas.fill_rect(x, y, w, 1, BORDER);
    canvas.fill_rect(x, y + h - 1, w, 1, BORDER);
    canvas.fill_rect(x, y, 1, h, BORDER);
    canvas.fill_rect(x + w - 1, y, 1, h, BORDER);
    let mut label = TextBuffer::with_metrics(font_system, 15.0, cosmic_text::Wrap::None);
    if !button.enabled {
        label.set_color(cosmic_text::Color::rgb(145, 145, 145));
    }
    label.set_text(font_system, button.label);
    label.set_size(font_system, w as f32, h as f32);
    label.draw(font_system, cache, canvas, x + 11, y + 5);
}

#[allow(clippy::too_many_arguments)]
fn paint_omnibox(
    font_system: &mut cosmic_text::FontSystem,
    cache: &mut cosmic_text::SwashCache,
    canvas: &mut Canvas<'_>,
    edit: &Omnibox,
    body_text: &mut TextBuffer,
    prefix_text: &mut TextBuffer,
    focused: bool,
    rect: (i32, i32, i32, i32),
) {
    let (x, y, w, h) = rect;
    canvas.fill_rect(x, y, w, h, VIEW_BG);
    let border = if focused { ACCENT } else { BORDER };
    canvas.fill_rect(x, y, w, 2, border);
    canvas.fill_rect(x, y + h - 2, w, 2, border);
    canvas.fill_rect(x, y, 2, h, border);
    canvas.fill_rect(x + w - 2, y, 2, h, border);
    if focused && edit.is_selected() {
        // Selection highlight behind the whole text.
        let sel_w = body_text.first_line_width(font_system).max(2.0);
        canvas.fill_rect(
            x + 8,
            y + 6,
            sel_w.min((w - 16).max(2) as f32) as i32,
            (h - 12).max(2),
            SELECT_BG,
        );
    }
    body_text.draw(font_system, cache, canvas, x + 8, y + 6);
    if focused && !edit.is_selected() {
        let prefix = edit.text()[..edit.caret()].to_owned();
        prefix_text.set_text(font_system, &prefix);
        prefix_text.set_size(font_system, (w - 16).max(10) as f32, h as f32);
        let caret_x = x + 8 + prefix_text.first_line_width(font_system) as i32;
        canvas.fill_rect(caret_x.min(x + w - 4), y + 6, 2, (h - 12).max(2), ACCENT);
    }
}

fn paint_scrollbar(
    canvas: &mut Canvas<'_>,
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    content_h: f32,
    scroll_y: f32,
) {
    let track_x = vx + vw - SCROLLBAR_W;
    canvas.fill_rect(track_x, vy, SCROLLBAR_W, vh, TRACK);
    if content_h <= vh as f32 {
        return;
    }
    let thumb_h = (vh as f32 * vh as f32 / content_h).max(24.0) as i32;
    let max_scroll = (content_h - vh as f32).max(1.0);
    let thumb_y = vy + ((vh - thumb_h) as f32 * (scroll_y / max_scroll)) as i32;
    canvas.fill_rect(track_x + 2, thumb_y, SCROLLBAR_W - 4, thumb_h, THUMB);
}

/// Human byte counts for progress/status.
fn format_bytes(n: usize) -> String {
    const KI: usize = 1024;
    const MI: usize = 1024 * 1024;
    if n >= MI {
        format!("{:.1} MiB", n as f64 / MI as f64)
    } else if n >= KI {
        format!("{:.1} KiB", n as f64 / KI as f64)
    } else {
        format!("{n} B")
    }
}

fn host_or_url(url: &Url) -> String {
    url.host_str()
        .map(str::to_owned)
        .unwrap_or_else(|| url.as_str().to_owned())
}

impl ApplicationHandler<Wake> for ShellApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("browser")
            .with_inner_size(PhysicalSize::new(1100, 750));
        #[cfg(target_os = "linux")]
        let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attrs, "browser", "browser",
        );
        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);
                self.start_gpu(window.clone());
                self.window = Some(window);
                self.request_redraw();
                self.initialize_navigation();
            }
            Err(err) => {
                self.fatal_init = Some(format!("window creation failed: {err}"));
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.size = size;
                self.content_measured_for = None;
                self.queue_restyle();
                if let Some(fetcher) = &self.fetcher {
                    fetcher.send(Command::DomEvent {
                        id: self.navigation_id,
                        event: crate::script_worker::DomCommand::Resize(self.environment()),
                    });
                }
                self.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                if self.profile.preferences.appearance.theme == "system" {
                    self.dark_theme = theme == winit::window::Theme::Dark;
                    self.queue_restyle();
                }
            }
            WindowEvent::RedrawRequested => {
                self.render();
                if self.fatal_init.is_some() {
                    event_loop.exit();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.on_key(&event);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_x = position.x;
                self.mouse_y = position.y;
                if self.dragging_scroll {
                    self.drag_scroll_to(position.y as i32);
                }
                if let Some(from) = self.browser.dragged_tab {
                    let to = self.browser.widgets.iter().find_map(|widget| {
                        let (x, y, w, h) = widget.rect;
                        if position.x >= x as f64
                            && position.x < (x + w) as f64
                            && position.y >= y as f64
                            && position.y < (y + h) as f64
                        {
                            if let Some(ChromeAction::Select(i)) = &widget.action {
                                return Some(*i);
                            }
                        }
                        None
                    });
                    if let Some(to) = to.filter(|to| *to != from) {
                        self.reorder_tab(from, to);
                        self.browser.dragged_tab = Some(to);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.on_wheel(&delta);
            }
            WindowEvent::PinchGesture { delta, .. } => self.zoom_by((delta as f32).exp()),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    self.on_click();
                } else {
                    self.dragging_scroll = false;
                    self.browser.dragged_tab = None;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                if self.mouse_y < TABS_H as f64 {
                    let tab = self.browser.widgets.iter().find_map(|widget| {
                        let (x, y, w, h) = widget.rect;
                        if self.mouse_x >= x as f64
                            && self.mouse_x < (x + w) as f64
                            && self.mouse_y >= y as f64
                            && self.mouse_y < (y + h) as f64
                        {
                            if let Some(ChromeAction::Select(i) | ChromeAction::Close(i)) =
                                &widget.action
                            {
                                return Some(*i);
                            }
                        }
                        None
                    });
                    if let Some(tab) = tab {
                        self.close_tab(tab);
                    }
                } else {
                    let (vx, vy, _, _) = self.viewport_rect();
                    if self.mouse_y >= vy as f64 {
                        if let Some(url) = self.hit_link(
                            self.mouse_x as f32 - vx as f32 - self.page_origin.0 + self.scroll_x,
                            self.mouse_y as f32 - vy as f32 + self.scroll_y - self.page_origin.1,
                        ) {
                            if let Err(error) = self.new_tab(Some(url.as_str())) {
                                self.browser.notice = error;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: Wake) {
        self.drain_browser();
        self.drain_fetcher();
        self.drain_layout();
        self.poll_gpu();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_requested {
            event_loop.exit();
            return;
        }
        event_loop.set_control_flow(ControlFlow::Wait);
        // Drain in case events arrived without a wake (e.g. startup fetch).
        self.drain_fetcher();
        self.drain_browser();
        self.drain_layout();
        self.poll_gpu();
    }
}
