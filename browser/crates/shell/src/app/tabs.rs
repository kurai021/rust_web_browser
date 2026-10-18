//! Per-tab presentation and navigation state. The GPU, fonts and chrome stay shared.
use super::*;

/// A provisional preview must never be paired with the previous page's VM.
/// Retain the committed presentation until commit, attachment or stop.
pub(super) struct Presentation {
    page: Option<Arc<Page>>,
    styles: ComputedStyles,
    article: Article,
    view: View,
    local: Option<LocalPage>,
    flow: Option<Arc<layout::LayoutResult>>,
    scene: paint::Scene,
    flow_navigation: Option<u64>,
    scroll_offsets: layout::ScrollOffsets,
    scroll: (f32, f32),
    zoom: f32,
    content_h: f32,
    canvas: css::values::Color,
    origin: (f32, f32),
    fonts: cosmic_text::fontdb::Database,
    font_ids: Vec<cosmic_text::fontdb::ID>,
    favicon: Option<Arc<layout::RasterImage>>,
}
impl Presentation {
    pub(super) fn capture(app: &ShellApp) -> Self {
        Self {
            page: app.page.clone(),
            styles: app.styles.clone(),
            article: app.article.clone(),
            view: app.view,
            local: app.local_page,
            flow: app.flow.clone(),
            scene: app.scene.clone(),
            flow_navigation: app.flow_navigation,
            scroll_offsets: app.scroll_offsets.clone(),
            scroll: (app.scroll_x, app.scroll_y),
            zoom: app.zoom,
            content_h: app.content_h,
            canvas: app.canvas_color,
            origin: app.page_origin,
            fonts: app.page_font_db.clone(),
            font_ids: app.web_font_ids.clone(),
            favicon: app.favicon.clone(),
        }
    }
    pub(super) fn restore(self, app: &mut ShellApp) {
        app.page = self.page;
        app.styles = self.styles;
        app.article = self.article;
        app.view = self.view;
        app.local_page = self.local;
        app.flow = self.flow;
        app.scene = self.scene;
        app.flow_navigation = self.flow_navigation;
        app.scroll_offsets = self.scroll_offsets;
        app.scroll_x = self.scroll.0;
        app.scroll_y = self.scroll.1;
        app.zoom = self.zoom;
        app.content_h = self.content_h;
        app.canvas_color = self.canvas;
        app.page_origin = self.origin;
        app.page_font_db = self.fonts;
        app.web_font_ids = self.font_ids;
        app.favicon = self.favicon;
        app.font_generation = app.font_generation.wrapping_add(1);
        app.content_version = app.content_version.wrapping_add(1);
        app.content_measured_for = None;
    }
}

pub(super) struct TabState {
    pub(super) committed: bool,
    pub(super) pending_traversal: i32,
    pub(super) previous_presentation: Option<Presentation>,
    pub(super) page_font_db: cosmic_text::fontdb::Database,
    pub(super) favicon: Option<Arc<layout::RasterImage>>,
    pub(super) crashed: bool,
    pub(super) previous_navigation: Option<(u64, bool)>,
    pub(super) navigation_url: Option<Url>,
    pub(super) local_page: Option<LocalPage>,
    pub(super) fetcher: Option<Fetcher>,
    pub(super) history: History,
    pub(super) omnibox: Omnibox,
    pub(super) omnibox_focused: bool,
    pub(super) view: View,
    pub(super) article: Article,
    pub(super) page: Option<Arc<Page>>,
    pub(super) redirected_from: Option<Url>,
    pub(super) styles: ComputedStyles,
    pub(super) navigation_id: u64,
    pub(super) style_revision: u64,
    pub(super) web_font_ids: Vec<cosmic_text::fontdb::ID>,
    pub(super) page_origin: (f32, f32),
    pub(super) canvas_color: css::values::Color,
    pub(super) blocks: Vec<BlockView>,
    pub(super) flow: Option<Arc<layout::LayoutResult>>,
    pub(super) scene: paint::Scene,
    pub(super) flow_navigation: Option<u64>,
    pub(super) scroll_offsets: layout::ScrollOffsets,
    pub(super) zoom: f32,
    pub(super) layout_passes: u64,
    pub(super) layout_ms: f64,
    pub(super) navigation_started: Option<std::time::Instant>,
    pub(super) painted_navigation: bool,
    pub(super) preview_seen: bool,
    pub(super) script_ready: bool,
    pub(super) dom_focused: Option<html::NodeId>,
    pub(super) script_dialog: Option<String>,
    pub(super) script_console: Vec<String>,
    pub(super) web_dialog: Option<crate::web_ui::WebDialog>,
    pub(super) web_dialogs: std::collections::VecDeque<crate::web_ui::WebDialog>,
    pub(super) spa_back: bool,
    pub(super) spa_forward: bool,
    pub(super) observer_viewport: Option<web_api::runtime::GeometryRect>,
    pub(super) load: LoadState,
    pub(super) pending_history: bool,
    pub(super) scroll_y: f32,
    pub(super) scroll_x: f32,
    pub(super) content_h: f32,
    pub(super) content_measured_for: Option<(u64, u32)>,
    pub(super) content_version: u64,
    pub(super) pending_restore: Option<profile::SavedTab>,
    pub(super) restore_scroll: Option<(f32, f32)>,
}
impl TabState {
    pub(super) fn blank(
        fetcher: Fetcher,
        navigation: u64,
        page_font_db: cosmic_text::fontdb::Database,
    ) -> Self {
        let mut history = History::new();
        if let Ok(blank) = Url::parse("about:blank") {
            history.navigate(blank);
        }
        Self {
            committed: false,
            pending_traversal: 0,
            previous_presentation: None,
            page_font_db,
            favicon: None,
            crashed: false,
            previous_navigation: None,
            navigation_url: None,
            local_page: None,
            fetcher: Some(fetcher),
            history,
            omnibox: Omnibox::new(),
            omnibox_focused: true,
            view: View::Welcome,
            article: welcome_article(),
            page: None,
            redirected_from: None,
            styles: ComputedStyles::default(),
            navigation_id: navigation,
            style_revision: 0,
            web_font_ids: Vec::new(),
            page_origin: (12.0, 10.0),
            canvas_color: css::values::Color::WHITE,
            blocks: Vec::new(),
            flow: None,
            scene: paint::Scene::default(),
            flow_navigation: None,
            scroll_offsets: Default::default(),
            zoom: 1.0,
            layout_passes: 0,
            layout_ms: 0.0,
            navigation_started: None,
            painted_navigation: false,
            preview_seen: false,
            script_ready: false,
            dom_focused: None,
            script_dialog: None,
            script_console: Vec::new(),
            web_dialog: None,
            web_dialogs: Default::default(),
            spa_back: false,
            spa_forward: false,
            observer_viewport: None,
            load: LoadState::Idle,
            pending_history: false,
            scroll_y: 0.0,
            scroll_x: 0.0,
            content_h: 0.0,
            content_measured_for: None,
            content_version: 1,
            pending_restore: None,
            restore_scroll: None,
        }
    }
    /// Move handles and retained display lists; never clone a realm or native DOM.
    pub(super) fn swap(&mut self, app: &mut ShellApp) {
        macro_rules! swap { ($($field:ident),* $(,)?) => { $(std::mem::swap(&mut self.$field, &mut app.$field);)* }; }
        swap!(
            committed,
            pending_traversal,
            previous_presentation,
            page_font_db,
            favicon,
            crashed,
            previous_navigation,
            navigation_url,
            local_page,
            fetcher,
            history,
            omnibox,
            omnibox_focused,
            view,
            article,
            page,
            redirected_from,
            styles,
            navigation_id,
            style_revision,
            web_font_ids,
            page_origin,
            canvas_color,
            blocks,
            flow,
            scene,
            flow_navigation,
            scroll_offsets,
            zoom,
            layout_passes,
            layout_ms,
            navigation_started,
            painted_navigation,
            preview_seen,
            script_ready,
            dom_focused,
            script_dialog,
            script_console,
            web_dialog,
            web_dialogs,
            spa_back,
            spa_forward,
            observer_viewport,
            load,
            pending_history,
            scroll_y,
            scroll_x,
            content_h,
            content_measured_for,
            content_version,
            pending_restore,
            restore_scroll
        );
    }
    fn saved(&self) -> profile::SavedTab {
        if let Some(saved) = &self.pending_restore {
            return saved.clone();
        }
        let mut saved = self.history.save();
        if let LoadState::Loading { url, .. } = &self.load {
            saved.current = url.to_string();
        }
        let (x, y) = self
            .restore_scroll
            .unwrap_or((self.scroll_x, self.scroll_y));
        saved.scroll_x = x;
        saved.scroll_y = y;
        saved
    }
}

impl ShellApp {
    pub(super) fn next_navigation(&mut self) -> u64 {
        self.next_navigation_id = self.next_navigation_id.wrapping_add(1);
        self.next_navigation_id
    }
    pub(super) fn swap_context(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        if let Some(mut tab) = self.tabs[index].take() {
            let old = self.active_tab;
            tab.swap(self);
            self.tabs[old] = Some(tab);
            self.active_tab = index;
        }
    }
    pub fn select_tab(&mut self, index: usize) {
        self.swap_context(index);
        self.browser.focused = None;
        self.browser.suggestion = None;
        self.browser.find_key = None;
        if let Some(url) = self.page.as_ref().map(|p| &p.url) {
            self.zoom = self
                .profile
                .data
                .zoom
                .get(&url.origin().ascii_serialization())
                .copied()
                .unwrap_or(1.0);
        }
        if let Some(saved) = self.pending_restore.take() {
            self.omnibox_focused = false;
            self.history = History::restore(&saved);
            self.restore_scroll = Some((saved.scroll_x, saved.scroll_y));
            self.navigate_saved(&saved.current);
        }
        self.content_measured_for = None;
        self.queue_restyle();
        self.update_title();
        self.request_redraw();
        self.save_session();
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::DomEvent {
                id: self.navigation_id,
                event: crate::script_worker::DomCommand::Resize(self.environment()),
            });
        }
    }
    pub fn new_tab(&mut self, input: Option<&str>) -> Result<(), String> {
        if self.tabs.len() >= profile::MAX_TABS {
            return Err("Tab limit (50) reached".into());
        }
        let wake = self.gpu_wake.clone().unwrap_or_else(|| Arc::new(|| {}));
        let fetcher =
            Fetcher::spawn_shared(self.shared.clone(), wake).map_err(|e| e.to_string())?;
        let navigation = self.next_navigation();
        self.tabs.push(Some(TabState::blank(
            fetcher,
            navigation,
            self.font_system.db().clone(),
        )));
        self.select_tab(self.tabs.len() - 1);
        if let Some(input) = input {
            self.omnibox_focused = false;
            self.navigate_input(input);
        }
        self.save_session();
        Ok(())
    }
    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let old = self.active_tab;
        self.swap_context(index);
        if let Some(dialog) = self.web_dialog.take() {
            self.resolve_web_dialog(dialog, 0);
        }
        for dialog in self.web_dialogs.drain(..) {
            if let crate::web_ui::Kind::Dialog { reply, .. } = dialog.kind {
                let _ = reply.send(None);
            }
        }
        self.clear_web_fonts();
        if let Some(fetcher) = self.fetcher.take() {
            fetcher.shutdown();
        }
        if self.tabs.len() == 1 {
            self.tabs.clear();
            self.exit_requested = true;
            self.save_session();
            return;
        }
        let next = if old != index {
            old
        } else if index + 1 < self.tabs.len() {
            index + 1
        } else {
            index - 1
        };
        self.swap_context(next);
        self.tabs.remove(index);
        if self.active_tab > index {
            self.active_tab -= 1;
        }
        self.select_tab(self.active_tab);
    }
    pub fn duplicate_tab(&mut self) -> Result<(), String> {
        let saved = self.saved_active();
        self.new_tab(None)?;
        self.history = History::restore(&saved);
        self.restore_scroll = Some((saved.scroll_x, saved.scroll_y));
        self.navigate_saved(&saved.current);
        self.save_session();
        Ok(())
    }
    pub fn reorder_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active_tab = if self.active_tab == from {
            to
        } else if from < self.active_tab && self.active_tab <= to {
            self.active_tab - 1
        } else if to <= self.active_tab && self.active_tab < from {
            self.active_tab + 1
        } else {
            self.active_tab
        };
        self.save_session();
        self.request_redraw();
    }
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
    pub fn active_tab(&self) -> usize {
        self.active_tab
    }
    fn saved_active(&self) -> profile::SavedTab {
        if let Some(saved) = &self.pending_restore {
            return saved.clone();
        }
        let mut saved = self.history.save();
        if let LoadState::Loading { url, .. } = &self.load {
            saved.current = url.to_string();
        }
        let (x, y) = self
            .restore_scroll
            .unwrap_or((self.scroll_x, self.scroll_y));
        saved.scroll_x = x;
        saved.scroll_y = y;
        saved
    }
    pub(super) fn save_session(&mut self) {
        if self.batch_visible.is_some() {
            self.session_dirty = true;
            return;
        }
        self.profile.data.session = profile::Session {
            tabs: self
                .tabs
                .iter()
                .map(|tab| {
                    tab.as_ref()
                        .map_or_else(|| self.saved_active(), TabState::saved)
                })
                .collect(),
            active: self.batch_visible.unwrap_or(self.active_tab),
        };
        self.profile.changed();
    }
    pub(super) fn restore_startup(&mut self) {
        let session = self.profile.data.session.clone();
        if self.restore_session
            && self.profile.preferences.general.restore_session
            && !session.tabs.is_empty()
        {
            for (index, saved) in session.tabs.iter().enumerate() {
                if index > 0 {
                    let wake = self.gpu_wake.clone().unwrap_or_else(|| Arc::new(|| {}));
                    let Ok(fetcher) = Fetcher::spawn_shared(self.shared.clone(), wake) else {
                        break;
                    };
                    let navigation = self.next_navigation();
                    self.tabs.push(Some(TabState::blank(
                        fetcher,
                        navigation,
                        self.font_system.db().clone(),
                    )));
                }
                if index == 0 {
                    self.history = History::restore(saved);
                    self.pending_restore = Some(saved.clone());
                } else if let Some(tab) = &mut self.tabs[index] {
                    tab.history = History::restore(saved);
                    tab.pending_restore = Some(saved.clone());
                    tab.omnibox.set_text(&saved.current);
                }
            }
            self.select_tab(session.active.min(self.tabs.len() - 1));
        } else if let Some(url) = self.start_url.take() {
            self.omnibox_focused = false;
            if matches!(url.scheme(), "http" | "https") {
                self.omnibox.set_text(url.as_str());
                self.start_fetch(url, self.start_allow_downgrade, true);
            } else {
                self.navigate_input(url.as_str());
            }
        } else if let Some(query) = self.start_search.take() {
            self.omnibox_focused = false;
            self.navigate_input(&query);
        } else {
            let homepage = self.profile.preferences.general.homepage.clone();
            self.navigate_input(&homepage);
        }
    }
    pub(super) fn navigate_saved(&mut self, input: &str) {
        if let Ok(url) = Url::parse(input) {
            if matches!(url.scheme(), "http" | "https") {
                self.omnibox.set_text(url.as_str());
                self.start_fetch(url, false, false);
                return;
            }
        }
        self.navigate_input(input);
    }
}
