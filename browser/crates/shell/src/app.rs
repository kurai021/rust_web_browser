//! Phase 1 window: toolbar + omnibox + raw-text viewport + status bar.
//!
//! Stack per plan/10 §10.4 and plan/03 §3.4: `winit` window, `softbuffer`
//! software pixels, `cosmic-text` glyphs. Network stays on the worker
//! thread (`fetcher.rs`); this file never blocks on I/O.

use std::num::NonZeroU32;
use std::sync::Arc;

use net::{Error as NetError, Fetched};
use softbuffer::{Context, Surface};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes};

use crate::fetcher::{resolve_input, Command, FetchEvent, Fetcher};
use crate::history::History;
use crate::omnibox::Omnibox;
use crate::text::{rgb, Canvas, TextBuffer, TextRenderer};

/// Wake signal from the network worker.
#[derive(Debug)]
pub enum Wake {
    /// New fetch events are queued; drain them.
    Fetch,
}

/// Layout metrics (physical pixels).
const TOOLBAR_H: i32 = 44;
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

const WELCOME_TEXT: &str = "\
browser — Phase 1

This window shows the raw fetched source of a page (HTML parsing lands in Phase 2).

Try:
  example.com
  https://example.com/
  about:blank (this page)

Shortcuts:
  Ctrl+L focus address bar · Enter load · Esc stop/unfocus
  Ctrl+R reload · Alt+Left/Right back/forward · wheel/PgUp/PgDn scroll";

/// What the viewport currently shows.
#[derive(Debug, Clone)]
enum View {
    Welcome,
    Document { text: String },
    Error { text: String },
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
    fetcher: Option<Fetcher>,
    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    size: PhysicalSize<u32>,
    fatal_init: Option<String>,

    font_system: cosmic_text::FontSystem,
    text_renderer: TextRenderer,
    viewport_text: TextBuffer,
    omnibox_text: TextBuffer,
    omnibox_prefix: TextBuffer,
    status_text: TextBuffer,

    history: History,
    omnibox: Omnibox,
    omnibox_focused: bool,
    view: View,
    load: LoadState,
    pending_history: bool,
    scroll_y: f32,
    content_h: f32,
    content_measured_for: Option<(u64, u32, u32)>,
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
        let viewport_text =
            TextBuffer::with_metrics(&mut font_system, 15.0, cosmic_text::Wrap::Word);
        let omnibox_text =
            TextBuffer::with_metrics(&mut font_system, 15.0, cosmic_text::Wrap::None);
        let omnibox_prefix =
            TextBuffer::with_metrics(&mut font_system, 15.0, cosmic_text::Wrap::None);
        let status_text = TextBuffer::with_metrics(&mut font_system, 13.0, cosmic_text::Wrap::None);
        let mut app = Self {
            fetcher: Some(fetcher),
            window: None,
            context: None,
            surface: None,
            size: PhysicalSize::new(1100, 750),
            fatal_init: None,
            font_system,
            text_renderer,
            viewport_text,
            omnibox_text,
            omnibox_prefix,
            status_text,
            history: History::new(),
            omnibox: Omnibox::new(),
            omnibox_focused: true,
            view: View::Welcome,
            load: LoadState::Idle,
            pending_history: false,
            scroll_y: 0.0,
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
        app.viewport_text
            .set_text(&mut app.font_system, WELCOME_TEXT);
        app
    }

    /// Take the startup error, if window/surface creation failed.
    pub fn take_fatal(&mut self) -> Option<String> {
        self.fatal_init.take()
    }

    /// Join the worker thread (saves HSTS). Called after the loop exits.
    pub fn shutdown(&mut self) {
        if let Some(fetcher) = self.fetcher.take() {
            fetcher.shutdown();
        }
    }

    // ---------- navigation ----------

    fn submit_omnibox(&mut self) {
        let input = self.omnibox.text().to_owned();
        self.omnibox_focused = false;
        self.navigate_input(&input);
    }

    fn navigate_input(&mut self, input: &str) {
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("about:blank") || trimmed.is_empty() {
            self.show_welcome();
            return;
        }
        match resolve_input(trimmed) {
            Ok((url, allow_downgrade)) => self.start_fetch(url, allow_downgrade, true),
            Err(query) => self.show_search_stub(&query),
        }
    }

    /// Begin a fetch. `record_history` is false for back/forward/reload
    /// (the entry already exists); the final URL is recorded on Done.
    fn start_fetch(&mut self, url: Url, allow_downgrade: bool, record_history: bool) {
        self.pending_history = record_history;
        self.load = LoadState::Loading {
            url: url.clone(),
            downloaded: 0,
            total: None,
        };
        self.update_title();
        self.request_redraw();
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::Navigate {
                url,
                allow_downgrade,
            });
        }
    }

    fn reload(&mut self) {
        if let Some(url) = self.history.current().cloned() {
            self.start_fetch(url, false, false);
        } else if matches!(self.view, View::Error { .. }) {
            self.show_welcome();
        }
    }

    fn go_back(&mut self) {
        if let Some(url) = self.history.go_back() {
            self.omnibox.set_text(url.as_str());
            self.start_fetch(url, false, false);
        }
    }

    fn go_forward(&mut self) {
        if let Some(url) = self.history.go_forward() {
            self.omnibox.set_text(url.as_str());
            self.start_fetch(url, false, false);
        }
    }

    fn stop_or_reload(&mut self) {
        if matches!(self.load, LoadState::Loading { .. }) {
            if let Some(fetcher) = &self.fetcher {
                fetcher.send(Command::Stop);
            }
        } else {
            self.reload();
        }
    }

    fn bump_content(&mut self) {
        self.content_version += 1;
        self.scroll_y = 0.0;
    }

    fn show_welcome(&mut self) {
        self.load = LoadState::Idle;
        self.view = View::Welcome;
        self.viewport_text
            .set_text(&mut self.font_system, WELCOME_TEXT);
        self.omnibox.set_text("");
        self.bump_content();
        self.update_title();
        self.request_redraw();
    }

    fn show_search_stub(&mut self, query: &str) {
        // Search engine wiring lands in Phase 9 (plan/01.3.6).
        let text = format!(
            "[SEARCH]\nQuery: {query}\n\nWeb search needs a configured search engine (Phase 9).\nType a URL like example.com instead."
        );
        self.load = LoadState::Idle;
        self.view = View::Error { text: text.clone() };
        self.viewport_text.set_text(&mut self.font_system, &text);
        self.bump_content();
        self.update_title();
        self.request_redraw();
    }

    fn on_fetch_event(&mut self, event: FetchEvent) {
        match event {
            FetchEvent::Started { url } => {
                self.load = LoadState::Loading {
                    url,
                    downloaded: 0,
                    total: None,
                };
                self.update_title();
            }
            FetchEvent::Progress { downloaded, total } => {
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
            FetchEvent::Done { url, result } => {
                self.load = LoadState::Idle;
                match result {
                    Ok(fetched) => self.show_document(&url, &fetched),
                    Err(err) => self.show_error(&url, &err),
                }
            }
            FetchEvent::Stopped => {
                self.load = LoadState::Idle;
                self.update_title();
            }
        }
        self.request_redraw();
    }

    fn show_document(&mut self, requested: &Url, fetched: &Fetched) {
        if self.pending_history {
            self.history.navigate(fetched.url.clone());
        }
        self.pending_history = false;
        let size = format_bytes(fetched.bytes.len());
        let ctype = fetched.content_type.as_deref().unwrap_or("unknown type");
        let mut text = format!(
            "URL: {}\nStatus: {} · {} · {}\n{}\n",
            fetched.url.as_str(),
            fetched.status,
            ctype,
            size,
            "─".repeat(60)
        );
        if requested.as_str() != fetched.url.as_str() {
            text.insert_str(0, &format!("(redirected from {})\n", requested.as_str()));
        }
        text.push_str(&String::from_utf8_lossy(&fetched.bytes));
        self.view = View::Document { text: text.clone() };
        self.viewport_text.set_text(&mut self.font_system, &text);
        self.omnibox.set_text(fetched.url.as_str());
        self.bump_content();
        self.update_title();
    }

    fn show_error(&mut self, url: &Url, err: &NetError) {
        self.pending_history = false;
        let (title, advice) = match err {
            NetError::Tls(tls) => (
                format!("[TLS ERROR — {tls}]"),
                "The connection is not private: the certificate cannot be trusted.\nGo Back (Alt+Left). A per-site bypass lives behind about:config (Phase 9).",
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
        let text = format!("{title}\nURL: {}\n\n{err}\n\n{advice}", url.as_str());
        self.view = View::Error { text: text.clone() };
        self.viewport_text.set_text(&mut self.font_system, &text);
        self.bump_content();
        self.update_title();
    }

    fn update_title(&self) {
        let Some(window) = &self.window else { return };
        let title = match &self.load {
            LoadState::Loading { url, .. } => {
                format!("browser — {} (loading…)", host_or_url(url))
            }
            LoadState::Idle => match self.history.current() {
                Some(url) => format!("browser — {}", host_or_url(url)),
                None => "browser".to_owned(),
            },
        };
        window.set_title(&title);
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn drain_fetcher(&mut self) {
        // Collect first to end the fetcher borrow before mutating self.
        let mut events = Vec::new();
        if let Some(fetcher) = &self.fetcher {
            fetcher.drain_events(|event| events.push(event));
        }
        for event in events {
            self.on_fetch_event(event);
        }
    }

    // ---------- input ----------

    fn on_key(&mut self, event: &KeyEvent) {
        if event.state == ElementState::Released {
            return;
        }
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();

        // Global shortcuts first.
        if ctrl && !alt {
            match &event.logical_key {
                Key::Character(ch) if ch.as_str() == "l" || ch.as_str() == "L" => {
                    self.omnibox_focused = true;
                    self.request_redraw();
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

        if self.omnibox_focused {
            self.on_key_omnibox(event);
        } else {
            self.on_key_viewport(event);
        }
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
                        self.omnibox.insert(text.as_str());
                        self.request_redraw();
                    }
                }
            }
        }
    }

    fn on_key_viewport(&mut self, event: &KeyEvent) {
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
                self.stop_or_reload();
            }
            _ => {}
        }
    }

    fn on_wheel(&mut self, delta: &MouseScrollDelta) {
        let dy = match delta {
            MouseScrollDelta::LineDelta(_, y) => -*y * LINE_SCROLL_PX,
            MouseScrollDelta::PixelDelta(pos) => -(pos.y as f32),
        };
        self.scroll_by(dy);
    }

    fn scroll_by(&mut self, dy: f32) {
        let max = (self.content_h - self.viewport_h()).max(0.0);
        self.scroll_y = (self.scroll_y + dy).clamp(0.0, max);
        self.request_redraw();
    }

    fn viewport_h(&self) -> f32 {
        (self.size.height as i32 - TOOLBAR_H - STATUS_H).max(50) as f32
    }

    fn on_click(&mut self) {
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
                    _ => self.stop_or_reload(),
                }
                return;
            }
        }
        // Omnibox focus.
        let (x, y, w, h) = layout.omnibox_rect;
        if mx >= x && mx < x + w && my >= y && my < y + h {
            self.omnibox_focused = true;
            self.request_redraw();
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
        self.omnibox_focused = false;
        self.request_redraw();
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
            ("<", self.history.can_go_back()),
            (">", self.history.can_go_forward()),
            (if loading { "[]" } else { "R" }, true),
        ] {
            buttons.push(Button {
                label,
                rect: (x, 6, BUTTON_W, TOOLBAR_H - 12),
                enabled,
            });
            x += BUTTON_W + BUTTON_GAP;
        }
        let omnibox_rect = (x, 6, (w - x - PAD).max(80), TOOLBAR_H - 12);
        ToolbarLayout {
            buttons,
            omnibox_rect,
        }
    }

    fn viewport_rect(&self) -> (i32, i32, i32, i32) {
        let w = self.size.width as i32;
        let h = self.size.height as i32;
        (0, TOOLBAR_H, w, (h - TOOLBAR_H - STATUS_H).max(50))
    }

    // ---------- rendering ----------

    fn render(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        // Always track the real window size: compositors (tiling, decorations)
        // may assign anything, and the startup guess is often wrong. A stale
        // size makes softbuffer fail and the window never commits a frame.
        self.size = window.inner_size();
        if self.surface.is_none() {
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
        self.measure_content(vw as f32, vh as f32);
        let max_scroll = (self.content_h - vh as f32).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        self.viewport_text
            .set_scroll_px(&mut self.font_system, self.scroll_y);
        let status = self.status_line();
        self.status_text.set_text(&mut self.font_system, &status);
        self.status_text
            .set_size(&mut self.font_system, w as f32, STATUS_H as f32);
        let layout = self.toolbar_layout();
        let omni_w = (layout.omnibox_rect.2 - 16).max(10) as f32;
        self.omnibox_text
            .set_text(&mut self.font_system, self.omnibox.text());
        self.omnibox_text
            .set_size(&mut self.font_system, omni_w, h as f32);

        // Phase B: pixels. `surface` stays inside `self`; every other
        // borrow below is a disjoint field, so this compiles without moves.
        // Surface errors are logged: failing silently here leaves a window
        // that never commits a frame (invisible to the compositor).
        let surface = self.surface.as_mut().expect("surface created above");
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
        let fs = &mut self.font_system;
        let cache = &mut self.text_renderer.cache;
        let mut canvas = Canvas {
            w,
            h,
            pixels: &mut buffer,
        };
        canvas.fill_rect(0, 0, w as i32, h as i32, BG);
        canvas.fill_rect(vx, vy, vw, vh, VIEW_BG);
        self.viewport_text
            .draw(fs, cache, &mut canvas, vx + 12, vy + 10);
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
            &self.omnibox_text,
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
        if let Err(err) = buffer.present() {
            eprintln!("browser: present failed: {err}");
        }
    }

    /// Re-measure laid-out content height only when content or width changed.
    fn measure_content(&mut self, vw: f32, vh: f32) {
        let key = (self.content_version, vw as u32, vh as u32);
        if self.content_measured_for == Some(key) {
            return;
        }
        self.viewport_text
            .set_size(&mut self.font_system, vw, vh.max(50.0));
        self.content_h = self.viewport_text.full_height(&mut self.font_system) + 20.0;
        self.content_measured_for = Some(key);
    }

    fn status_line(&self) -> String {
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
            LoadState::Idle => match &self.view {
                View::Welcome => "Ready — type a URL and press Enter".to_owned(),
                View::Document { text } | View::Error { text } => {
                    let lines = text.lines().count();
                    format!("Done — {lines} lines (raw source; HTML parsing lands in Phase 2)")
                }
            },
        }
    }
}

struct ToolbarLayout {
    buttons: Vec<Button>,
    omnibox_rect: (i32, i32, i32, i32),
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
    if !button.enabled {
        return;
    }
    let mut label = TextBuffer::with_metrics(font_system, 15.0, cosmic_text::Wrap::None);
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
    body_text: &TextBuffer,
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
    body_text.draw(font_system, cache, canvas, x + 8, y + 6);
    if focused {
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
        match event_loop.create_window(attrs) {
            Ok(window) => {
                self.window = Some(Arc::new(window));
                self.request_redraw();
                // Kick off the startup navigation/search now that the UI exists.
                if let Some(url) = self.start_url.take() {
                    let allow_downgrade = self.start_allow_downgrade;
                    self.omnibox.set_text(url.as_str());
                    self.start_fetch(url, allow_downgrade, true);
                } else if let Some(query) = self.start_search.take() {
                    self.show_search_stub(&query);
                }
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
                self.request_redraw();
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
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.on_wheel(&delta);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    self.on_click();
                } else {
                    self.dragging_scroll = false;
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: Wake) {
        self.drain_fetcher();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // Drain in case events arrived without a wake (e.g. startup fetch).
        self.drain_fetcher();
    }
}
