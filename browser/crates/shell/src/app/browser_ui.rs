//! Trusted browser chrome. These controls are never DOM nodes or page scripts.
use super::*;
use crate::{downloads, file_ops};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LocalPage {
    Preferences,
    History,
    Bookmarks,
    Downloads,
    Devtools,
}
impl LocalPage {
    pub(super) fn path(self) -> &'static str {
        match self {
            Self::Preferences => "about:preferences",
            Self::History => "about:history",
            Self::Bookmarks => "about:bookmarks",
            Self::Downloads => "about:downloads",
            Self::Devtools => "about:devtools",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Preferences => "Preferences",
            Self::History => "Local history",
            Self::Bookmarks => "Bookmarks",
            Self::Downloads => "Downloads",
            Self::Devtools => "Developer tools",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Homepage,
    Search,
    Directory,
    Filter,
    Import,
    Save,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Select(usize),
    Close(usize),
    New,
    Duplicate,
    Home,
    Security,
    Menu,
    Navigate(String),
    Local(LocalPage),
    Bookmark,
    RemoveBookmark(String),
    ClearHistory(u8),
    Edit(Field),
    Toggle(u8),
    Next(i32),
    Pause(u64),
    Resume(u64),
    Cancel(u64),
    Folder(u64),
    Save,
    Print,
    Export,
    Commit,
    Dismiss,
    FindNext(i32),
    FindClose,
    FindFocus,
    Suggest(String),
    Devtools(u8),
}
pub(super) struct Widget {
    pub(super) rect: (i32, i32, i32, i32),
    pub(super) label: String,
    pub(super) action: Option<Action>,
    pub(super) overlay: bool,
    pub(super) selected: bool,
    pub(super) size: f32,
}
impl Widget {
    fn label(rect: (i32, i32, i32, i32), label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            action: None,
            overlay: false,
            selected: false,
            size: 15.0,
        }
    }
    fn button(rect: (i32, i32, i32, i32), label: impl Into<String>, action: Action) -> Self {
        Self {
            action: Some(action),
            ..Self::label(rect, label)
        }
    }
    pub(super) fn draw(
        &self,
        fs: &mut cosmic_text::FontSystem,
        cache: &mut cosmic_text::SwashCache,
        canvas: &mut Canvas<'_>,
        dark: bool,
    ) {
        let (x, y, w, h) = self.rect;
        let background = if self.selected {
            0xc6def4
        } else if dark {
            0x243347
        } else {
            0xf3f6fa
        };
        if self.action.is_some() || self.overlay {
            canvas.fill_rect(x, y, w, h, background);
            canvas.fill_rect(x, y + h - 1, w, 1, if dark { 0x486079 } else { BORDER });
        }
        let limit = ((w - 14).max(1) as f32 / (self.size * 0.5)).max(1.0) as usize;
        let limit = limit * ((h as f32 / (self.size * 1.3)).floor() as usize).max(1);
        let label: String = self.label.chars().take(limit.min(300)).collect();
        let mut text = TextBuffer::with_metrics(fs, self.size, cosmic_text::Wrap::WordOrGlyph);
        text.set_color(if dark && !self.selected {
            cosmic_text::Color::rgb(230, 238, 247)
        } else {
            cosmic_text::Color::rgb(30, 47, 66)
        });
        text.set_text(fs, &label);
        text.set_size(fs, (w - 12).max(1) as f32, (h - 4).max(1) as f32);
        text.draw(fs, cache, canvas, x + 6, y + 4);
    }
}
enum SaveJob {
    Download(downloads::Source),
    Pdf(paint::Scene, layout::Size),
    Bookmarks,
}
enum UiEvent {
    Imported(Result<Vec<profile::Bookmark>, String>),
    Saved(Result<PathBuf, String>),
    Clipboard(Result<String, web_api::ApiError>),
    Notice(Result<String, String>),
}
struct Inspection {
    document: Arc<html::Document>,
    styles: ComputedStyles,
    console: Vec<String>,
    source: usize,
}
pub(super) struct BrowserUi {
    pub(super) widgets: Vec<Widget>,
    pub(super) menu: bool,
    pub(super) editor: Option<Field>,
    pub(super) edit: Omnibox,
    pub(super) filter: Omnibox,
    pub(super) focused: Option<usize>,
    pub(super) notice: String,
    pub(super) dragged_tab: Option<usize>,
    pub(super) suggestion: Option<usize>,
    pub(super) find_open: bool,
    pub(super) find_focused: bool,
    pub(super) find_query: Omnibox,
    pub(super) find_matches: Vec<crate::find::Match>,
    pub(super) find_active: usize,
    find_ticket: u64,
    pub(super) find_key: Option<(u64, u64, String)>,
    pub(super) find_worker: Option<crate::find::Worker>,
    pub(super) downloads: Option<downloads::Manager>,
    rates: std::collections::BTreeMap<u64, (u64, Option<u64>)>,
    save: Option<SaveJob>,
    site: Option<String>,
    site_origin: Option<String>,
    offset: usize,
    devtools: u8,
    inspection: Option<Inspection>,
    sender: std::sync::mpsc::Sender<UiEvent>,
    receiver: std::sync::mpsc::Receiver<UiEvent>,
    io: Option<std::thread::JoinHandle<()>>,
    clipboard_pending: bool,
}
impl Default for BrowserUi {
    fn default() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            widgets: Vec::new(),
            menu: false,
            editor: None,
            edit: Omnibox::new(),
            filter: Omnibox::new(),
            focused: None,
            notice: String::new(),
            dragged_tab: None,
            suggestion: None,
            find_open: false,
            find_focused: false,
            find_query: Omnibox::new(),
            find_matches: Vec::new(),
            find_active: 0,
            find_ticket: 0,
            find_key: None,
            find_worker: None,
            downloads: None,
            rates: Default::default(),
            save: None,
            site: None,
            site_origin: None,
            offset: 0,
            devtools: 0,
            inspection: None,
            sender,
            receiver,
            io: None,
            clipboard_pending: false,
        }
    }
}
impl ShellApp {
    pub(super) fn open_local(&mut self, page: LocalPage) {
        self.previous_presentation = None;
        self.previous_navigation = None;
        if let Some(url) = self.page.as_ref().map(|p| &p.url) {
            self.browser.site = url.host_str().map(str::to_owned);
            self.browser.site_origin = Some(url.origin().ascii_serialization());
        }
        if let Some(fetcher) = &self.fetcher {
            fetcher.send(Command::Stop {
                id: self.navigation_id,
            });
        }
        self.navigation_id = self.next_navigation();
        self.script_ready = false;
        self.load = LoadState::Idle;
        self.set_article(
            Article {
                title: Some(page.title().into()),
                ..Default::default()
            },
            View::Welcome,
        );
        self.local_page = Some(page);
        self.browser.offset = 0;
        self.browser.menu = false;
        self.browser.filter.set_text("");
        self.browser.editor = None;
        if let Ok(url) = Url::parse(page.path()) {
            if self.history.current() != Some(&url) {
                self.history.navigate(url);
            }
        }
        self.restore_scroll = None;
        self.navigation_url = None;
        self.omnibox.set_text(page.path());
        self.omnibox_focused = false;
        self.update_title();
        self.save_session();
        self.request_redraw();
    }
    pub(super) fn chrome_action(&mut self, action: Action) {
        self.browser.menu = false;
        let result = match action {
            Action::Select(i) => {
                self.select_tab(i);
                Ok(())
            }
            Action::Close(i) => {
                self.close_tab(i);
                Ok(())
            }
            Action::New => self.new_tab(None),
            Action::Duplicate => self.duplicate_tab(),
            Action::Home => {
                let home = self.profile.preferences.general.homepage.clone();
                self.navigate_input(&home);
                Ok(())
            }
            Action::Local(LocalPage::Devtools) => {
                self.open_devtools();
                Ok(())
            }
            Action::Local(page) => {
                self.open_local(page);
                Ok(())
            }
            Action::Navigate(input) | Action::Suggest(input) => {
                self.omnibox_focused = false;
                self.navigate_input(&input);
                Ok(())
            }
            Action::Security => {
                self.security_dialog();
                Ok(())
            }
            Action::Menu => {
                self.browser.menu = true;
                self.browser.focused = None;
                Ok(())
            }
            Action::Bookmark => {
                if let Some(url) = self.history.current() {
                    self.profile
                        .data
                        .bookmark(url, self.article.title.as_deref().unwrap_or(url.as_str()));
                    self.profile.changed();
                }
                Ok(())
            }
            Action::RemoveBookmark(url) => {
                self.profile
                    .data
                    .bookmarks
                    .retain(|b| b.url.as_str() != url);
                self.profile.changed();
                Ok(())
            }
            Action::ClearHistory(mode) => {
                let since = if mode == 1 {
                    profile::now().saturating_sub(3600)
                } else {
                    0
                };
                let site = if mode == 2 {
                    self.browser.site.as_deref()
                } else {
                    None
                };
                self.profile.data.delete_history(since, site);
                self.profile.changed();
                Ok(())
            }
            Action::Toggle(option) => {
                self.toggle_preference(option);
                Ok(())
            }
            Action::Edit(field) => {
                self.edit_chrome(field);
                Ok(())
            }
            Action::Commit => self.commit_editor(),
            Action::Dismiss => {
                self.browser.editor = None;
                self.browser.save = None;
                self.browser.menu = false;
                Ok(())
            }
            Action::Next(delta) => {
                self.browser.offset = if delta < 0 {
                    self.browser.offset.saturating_sub(10)
                } else {
                    self.browser.offset.saturating_add(10)
                };
                Ok(())
            }
            Action::Pause(id) => self
                .browser
                .downloads
                .as_ref()
                .map_or(Ok(()), |d| d.pause(id)),
            Action::Resume(id) => self
                .browser
                .downloads
                .as_ref()
                .map_or(Ok(()), |d| d.resume(id)),
            Action::Cancel(id) => self
                .browser
                .downloads
                .as_ref()
                .map_or(Ok(()), |d| d.cancel(id)),
            Action::Folder(id) => {
                if let Some(folder) = self
                    .profile
                    .data
                    .downloads
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.destination.parent())
                    .and_then(|p| Url::from_directory_path(p).ok())
                {
                    self.spawn_ui_io(move || {
                        UiEvent::Notice(
                            crate::web_ui::open_folder(&folder)
                                .map(|()| "Folder opened".into())
                                .map_err(|e| e.to_string()),
                        )
                    });
                }
                Ok(())
            }
            Action::Save => {
                if let Some(page) = &self.page {
                    let source = downloads::Source {
                        force_prompt: false,
                        url: page.url.clone(),
                        top: page.url.clone(),
                        filename: downloads::suggested_filename(&page.url, None),
                        bytes: None,
                    };
                    self.offer_download(source);
                }
                Ok(())
            }
            Action::Print => {
                if let Some(flow) = &self.flow {
                    let path = profile::expand_home(&self.profile.preferences.downloads.path)
                        .join("page.pdf");
                    self.browser.save = Some(SaveJob::Pdf(self.scene.clone(), flow.content_size));
                    self.browser.edit.set_text(&path.to_string_lossy());
                    self.browser.edit.select_all();
                    self.browser.editor = Some(Field::Save);
                } else {
                    self.browser.notice = "Load a rendered web document before printing".into();
                }
                Ok(())
            }
            Action::Export => {
                let path = profile::expand_home(&self.profile.preferences.downloads.path)
                    .join("bookmarks.html");
                self.browser.save = Some(SaveJob::Bookmarks);
                self.browser.edit.set_text(&path.to_string_lossy());
                self.browser.edit.select_all();
                self.browser.editor = Some(Field::Save);
                Ok(())
            }
            Action::FindNext(delta) => {
                self.find_next(delta);
                Ok(())
            }
            Action::FindClose => {
                self.browser.find_open = false;
                self.browser.find_focused = false;
                self.content_measured_for = None;
                Ok(())
            }
            Action::FindFocus => {
                self.browser.find_focused = true;
                self.browser.find_query.select_all();
                self.omnibox_focused = false;
                Ok(())
            }
            Action::Devtools(tab) => {
                self.browser.devtools = tab;
                self.browser.offset = 0;
                Ok(())
            }
        };
        if let Err(error) = result {
            self.browser.notice = error;
        }
        self.request_redraw();
    }
    fn toggle_preference(&mut self, option: u8) {
        let prefs = &mut self.profile.preferences;
        match option {
            0 => prefs.general.restore_session = !prefs.general.restore_session,
            1 => prefs.downloads.ask_every_time = !prefs.downloads.ask_every_time,
            2 => {
                prefs.appearance.theme = match prefs.appearance.theme.as_str() {
                    "system" => "light",
                    "light" => "dark",
                    _ => "system",
                }
                .into();
                self.dark_theme = prefs.appearance.theme == "dark"
                    || prefs.appearance.theme == "system"
                        && self.window.as_ref().and_then(|w| w.theme())
                            == Some(winit::window::Theme::Dark);
            }
            3 => prefs.privacy.javascript_enabled = !prefs.privacy.javascript_enabled,
            5 => {
                prefs.privacy.third_party_cookies = if prefs.privacy.third_party_cookies == "block"
                {
                    "allow"
                } else {
                    "block"
                }
                .into();
                self.shared
                    .client
                    .cookies
                    .set_block_third_party(prefs.privacy.third_party_cookies == "block");
            }
            6 => prefs.privacy.doh_enabled = !prefs.privacy.doh_enabled,
            4 => {
                if let Some(key) = &self.browser.site_origin {
                    let enabled = prefs
                        .privacy
                        .site_javascript
                        .get(key)
                        .copied()
                        .unwrap_or(prefs.privacy.javascript_enabled);
                    prefs.privacy.site_javascript.insert(key.clone(), !enabled);
                }
            }
            _ => {}
        }
        self.profile.changed();
        self.browser.notice = if option == 6 {
            "DNS preference saved. Restart the browser to apply it; no startup queries are sent."
        } else {
            "Preferences saved locally. JavaScript changes apply on the site's next reload."
        }
        .into();
        self.queue_restyle();
    }
    fn edit_chrome(&mut self, field: Field) {
        let value = match field {
            Field::Homepage => self.profile.preferences.general.homepage.clone(),
            Field::Search => self.profile.preferences.general.search_engine.clone(),
            Field::Directory => self.profile.preferences.downloads.path.clone(),
            Field::Filter => self.browser.filter.text().into(),
            Field::Import => profile::expand_home("~/Downloads/bookmarks.html")
                .to_string_lossy()
                .into_owned(),
            Field::Save => self.browser.edit.text().into(),
        };
        self.browser.edit.set_text(&value);
        self.browser.edit.select_all();
        self.browser.editor = Some(field);
        self.omnibox_focused = false;
    }
    fn commit_editor(&mut self) -> Result<(), String> {
        let Some(field) = self.browser.editor.clone() else {
            return Ok(());
        };
        let value = self.browser.edit.text().to_owned();
        if matches!(field, Field::Import | Field::Save)
            && self.browser.io.as_ref().is_some_and(|w| !w.is_finished())
        {
            return Err("Another file operation is still running; retry when it finishes".into());
        }
        match field {
            Field::Homepage | Field::Search | Field::Directory => {
                let mut prefs = self.profile.preferences.clone();
                match field {
                    Field::Homepage => prefs.general.homepage = value,
                    Field::Search => prefs.general.search_engine = value,
                    _ => prefs.downloads.path = value,
                }
                prefs.validate()?;
                self.profile.preferences = prefs;
                self.profile.changed();
                self.browser.notice = "Preferences saved".into();
            }
            Field::Filter => {
                self.browser.filter.set_text(&value);
                self.browser.offset = 0;
            }
            Field::Import => {
                let path = profile::expand_home(&value);
                self.spawn_ui_io(move || UiEvent::Imported(file_ops::read_bookmarks(&path)));
            }
            Field::Save => {
                let path = profile::expand_home(&value);
                let job = self.browser.save.take().ok_or("No pending save")?;
                match job {
                    SaveJob::Download(source) => {
                        let manager = self
                            .browser
                            .downloads
                            .as_ref()
                            .ok_or("Downloads worker unavailable")?;
                        if let Err(error) = manager.enqueue(source.clone(), path) {
                            self.browser.save = Some(SaveJob::Download(source));
                            return Err(error);
                        }
                        self.browser.notice = "Download queued; Ctrl+J shows progress".into();
                    }
                    SaveJob::Pdf(scene, size) => self.spawn_ui_io(move || {
                        UiEvent::Saved(
                            file_ops::print_pdf(&scene, size)
                                .and_then(|bytes| file_ops::save_export(&path, &bytes)),
                        )
                    }),
                    SaveJob::Bookmarks => {
                        let entries = self.profile.data.bookmarks.clone();
                        self.spawn_ui_io(move || {
                            UiEvent::Saved(file_ops::save_export(
                                &path,
                                file_ops::export_bookmarks(&entries).as_bytes(),
                            ))
                        });
                    }
                }
            }
        }
        self.browser.editor = None;
        Ok(())
    }
    pub(super) fn offer_download(&mut self, source: downloads::Source) {
        if self.browser.save.is_some() {
            self.browser.notice =
                "A save dialog is already open; finish it before starting another download".into();
            return;
        }
        let directory = self.profile.preferences.downloads.path.clone();
        let destination = profile::expand_home(if directory.is_empty() {
            "~/Downloads"
        } else {
            &directory
        })
        .join(downloads::sanitize_filename(&source.filename));
        if source.force_prompt
            || self.profile.preferences.downloads.ask_every_time
            || directory.is_empty()
        {
            self.browser.save = Some(SaveJob::Download(source));
            self.browser.edit.set_text(&destination.to_string_lossy());
            self.browser.edit.select_all();
            self.browser.editor = Some(Field::Save);
            self.omnibox_focused = false;
        } else if let Some(manager) = &self.browser.downloads {
            if let Err(error) = manager.enqueue(source, destination) {
                self.browser.notice = error;
            } else {
                self.browser.notice = "Download queued; Ctrl+J shows progress".into();
            }
        }
        self.request_redraw();
    }
    fn spawn_ui_io(&mut self, job: impl FnOnce() -> UiEvent + Send + 'static) {
        if self.browser.io.as_ref().is_some_and(|w| !w.is_finished()) {
            self.browser.notice = "Another file operation is still running".into();
            return;
        }
        if let Some(worker) = self.browser.io.take() {
            let _ = worker.join();
        }
        let sender = self.browser.sender.clone();
        let wake = self.gpu_wake.clone();
        match std::thread::Builder::new()
            .name("browser-file-operation".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                    .unwrap_or_else(|_| {
                        UiEvent::Saved(Err("File operation stopped unexpectedly".into()))
                    });
                let _ = sender.send(result);
                if let Some(wake) = wake {
                    wake();
                }
            }) {
            Ok(worker) => self.browser.io = Some(worker),
            Err(error) => self.browser.notice = error.to_string(),
        }
    }
    fn security_dialog(&mut self) {
        let url = self
            .page
            .as_ref()
            .map(|p| &p.url)
            .or_else(|| self.history.current());
        let message=match url{Some(url)if url.scheme()=="https"=>self.shared.client.certificate_details(url).unwrap_or_else(||format!("Connection to {}\nTLS certificate and hostname verified by rustls using system and bundled roots. TLS 1.2 or newer; no certificate bypass.\n{}",url.host_str().unwrap_or(""),url)),Some(url)if url.scheme()=="http"=>format!("Not secure — HTTP is not encrypted.\n{url}"),_=>"This is a local browser page; no network connection.".into()};
        self.queue_web_dialog(crate::web_ui::WebDialog::notice(message));
    }
    pub(super) fn drain_browser(&mut self) {
        let updates = self
            .browser
            .downloads
            .as_ref()
            .map_or_else(Vec::new, |m| m.drain());
        let changed = !updates.is_empty();
        for update in updates {
            let id = update.download.id;
            self.browser
                .rates
                .insert(id, (update.bytes_per_second, update.eta));
            if let Some(row) = self.profile.data.downloads.iter_mut().find(|d| d.id == id) {
                *row = update.download;
            } else {
                if self.profile.data.downloads.len() >= profile::MAX_DOWNLOADS {
                    self.profile.data.downloads.remove(0);
                }
                self.profile.data.downloads.push(update.download);
            }
        }
        if changed {
            self.profile.changed();
            self.request_redraw();
        }
        while let Ok(event) = self.browser.receiver.try_recv() {
            match event {
                UiEvent::Imported(result) => match result {
                    Ok(entries) => {
                        let mut added = 0;
                        for entry in entries {
                            if self.profile.data.bookmarks.len() >= profile::MAX_BOOKMARKS {
                                break;
                            }
                            if !self
                                .profile
                                .data
                                .bookmarks
                                .iter()
                                .any(|b| b.url == entry.url)
                            {
                                self.profile.data.bookmarks.push(entry);
                                added += 1;
                            }
                        }
                        self.profile.changed();
                        self.browser.notice = format!(
                            "Imported {added} bookmarks; source scripts were never executed"
                        );
                    }
                    Err(error) => self.browser.notice = error,
                },
                UiEvent::Saved(result) => {
                    self.browser.notice =
                        result.map_or_else(|e| e, |p| format!("Saved: {}", p.display()))
                }
                UiEvent::Notice(result) => self.browser.notice = result.unwrap_or_else(|e| e),
                UiEvent::Clipboard(result) => {
                    self.browser.clipboard_pending = false;
                    match result {
                        Ok(text) => {
                            let text: String = text
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(8192)
                                .collect();
                            if self.browser.editor.is_some() {
                                self.browser.edit.insert(&text);
                            } else if self.browser.find_focused {
                                self.browser.find_query.insert(&text);
                                self.request_find();
                            } else if self.omnibox_focused {
                                self.omnibox.insert(&text);
                            }
                        }
                        Err(error) => self.browser.notice = error.to_string(),
                    }
                }
            }
            self.request_redraw();
        }
        if let Some(error) = self.profile.take_error() {
            self.browser.notice = format!("Profile could not be saved: {error}");
            self.request_redraw();
        }
        let ready = self
            .browser
            .find_worker
            .as_ref()
            .map_or_else(Vec::new, |w| w.drain());
        for result in ready {
            if result.ticket == self.browser.find_ticket {
                self.browser.find_matches = result.matches;
                self.browser.find_active = 0;
                self.reveal_find();
                self.request_redraw();
            }
        }
    }
    pub(super) fn paste_chrome(&mut self) {
        if self.browser.clipboard_pending {
            return;
        }
        let sender = self.browser.sender.clone();
        let wake = self.gpu_wake.clone();
        match crate::web_ui::spawn_clipboard(None, move |result| {
            let _ = sender.send(UiEvent::Clipboard(result));
            if let Some(wake) = wake {
                wake();
            }
        }) {
            Ok(()) => self.browser.clipboard_pending = true,
            Err(error) => self.browser.notice = error.to_string(),
        }
    }
    pub(super) fn request_find(&mut self) {
        if !self.browser.find_open {
            return;
        }
        if self.flow.is_none() {
            self.browser.find_key = None;
            self.browser.find_matches.clear();
            return;
        }
        let key = (
            self.navigation_id,
            self.content_version,
            self.browser.find_query.text().to_owned(),
        );
        if self.browser.find_key.as_ref() == Some(&key) {
            return;
        }
        self.browser.find_key = Some(key.clone());
        self.browser.find_ticket = self.browser.find_ticket.wrapping_add(1);
        self.browser.find_matches.clear();
        self.browser.find_active = 0;
        if let (Some(worker), Some(flow)) = (&self.browser.find_worker, &self.flow) {
            worker.request(self.browser.find_ticket, flow.clone(), key.2);
        }
    }
    fn find_next(&mut self, delta: i32) {
        let len = self.browser.find_matches.len();
        if len > 0 {
            self.browser.find_active =
                (self.browser.find_active as i64 + delta as i64).rem_euclid(len as i64) as usize;
            self.reveal_find();
        }
        self.request_redraw();
    }
    fn reveal_find(&mut self) {
        let Some(hit) = self.browser.find_matches.get(self.browser.find_active) else {
            return;
        };
        let Some(part) = hit.parts.first() else {
            return;
        };
        let Some(flow) = &self.flow else {
            return;
        };
        let mut index = Some(part.box_index);
        let mut rect = part.rect;
        while let Some(i) = index {
            let Some(b) = flow.boxes.get(i) else {
                break;
            };
            if matches!(
                b.overflow,
                layout::Overflow::Auto | layout::Overflow::Scroll
            ) {
                let y = (rect.y - b.padding_box.y)
                    .max(0.0)
                    .min((b.scroll_size.height - b.padding_box.height).max(0.0));
                let x = (rect.x - b.padding_box.x)
                    .max(0.0)
                    .min((b.scroll_size.width - b.padding_box.width).max(0.0));
                self.scroll_offsets.insert(b.element, (x, y));
                rect.x -= x;
                rect.y -= y;
            }
            index = b.parent;
        }
        self.scroll_y = (rect.y * self.zoom - self.viewport_h() * 0.35).max(0.0);
        self.send_geometry();
    }
    pub(super) fn find_quads(&self) -> Vec<(paint::Quad, layout::Rect)> {
        if !self.browser.find_open {
            return Vec::new();
        }
        let Some(flow) = &self.flow else {
            return Vec::new();
        };
        let (vx, vy, vw, vh) = self.viewport_rect();
        let clip = layout::Rect::new(vx as f32, vy as f32, (vw - SCROLLBAR_W) as f32, vh as f32);
        let mut quads = Vec::new();
        for (index, hit) in self.browser.find_matches.iter().enumerate() {
            for part in &hit.parts {
                let mut rect = part.rect;
                let mut ancestor = Some(part.box_index);
                let mut fixed = false;
                while let Some(i) = ancestor {
                    let Some(b) = flow.boxes.get(i) else {
                        break;
                    };
                    if let Some((x, y)) = self.scroll_offsets.get(&b.element) {
                        rect.x -= x;
                        rect.y -= y;
                    }
                    fixed |= b.fixed;
                    ancestor = b.parent;
                }
                rect.x = rect.x * self.zoom + vx as f32 - if fixed { 0.0 } else { self.scroll_x };
                rect.y = rect.y * self.zoom + vy as f32 - if fixed { 0.0 } else { self.scroll_y };
                rect.width *= self.zoom;
                rect.height *= self.zoom;
                let color = if index == self.browser.find_active {
                    css::values::Color {
                        r: 255,
                        g: 155,
                        b: 20,
                        a: 100,
                    }
                } else {
                    css::values::Color {
                        r: 255,
                        g: 225,
                        b: 30,
                        a: 65,
                    }
                };
                quads.push((paint::Quad::solid(rect, color), clip));
            }
        }
        quads
    }
    pub(super) fn chrome_widgets(&self) -> Vec<Widget> {
        let mut widgets = Vec::new();
        let w = self.size.width as i32;
        let h = self.size.height as i32;
        let tab_width = ((w - 38).max(1) / self.tabs.len().max(1) as i32).clamp(64, 190);
        let visible = ((w - 38).max(1) / tab_width).max(1) as usize;
        let first = self.active_tab.saturating_sub(visible - 1);
        for (index, tab) in self.tabs.iter().enumerate().skip(first).take(visible) {
            let (title, loading) = if let Some(tab) = tab {
                (
                    tab.pending_restore
                        .as_ref()
                        .map(|saved| saved.current.as_str())
                        .or(tab.article.title.as_deref())
                        .unwrap_or("New tab"),
                    matches!(tab.load, LoadState::Loading { .. }),
                )
            } else {
                (
                    self.article.title.as_deref().unwrap_or("New tab"),
                    matches!(self.load, LoadState::Loading { .. }),
                )
            };
            let x = (index - first) as i32 * tab_width;
            let mut widget = Widget::button(
                (x, 0, tab_width - 22, TABS_H - 2),
                format!("{}{}", if loading { "… " } else { "    " }, title),
                Action::Select(index),
            );
            widget.selected = index == self.active_tab;
            widgets.push(widget);
            widgets.push(Widget::button(
                (x + tab_width - 22, 0, 22, TABS_H - 2),
                "×",
                Action::Close(index),
            ));
        }
        widgets.push(Widget::button(
            (
                (visible.min(self.tabs.len()) as i32 * tab_width).min(w - 34),
                0,
                32,
                TABS_H - 2,
            ),
            "+",
            Action::New,
        ));
        for (index, b) in self
            .profile
            .data
            .bookmarks
            .iter()
            .take(((w - 100).max(0) / 140) as usize)
            .enumerate()
        {
            widgets.push(Widget::button(
                (8 + index as i32 * 140, TABS_H + NAV_H, 136, BOOKMARKS_H - 2),
                &b.title,
                Action::Navigate(b.url.to_string()),
            ));
        }
        widgets.push(Widget::button(
            (w - 96, TABS_H + NAV_H, 88, BOOKMARKS_H - 2),
            "Bookmarks",
            Action::Local(LocalPage::Bookmarks),
        ));
        if let Some(page) = self.local_page {
            let y = TOOLBAR_H + 16;
            let mut title = Widget::label((22, y, w - 44, 34), page.title());
            title.size = 24.0;
            widgets.push(title);
            match page {
                LocalPage::Preferences => {
                    let p = &self.profile.preferences;
                    let mut line = y + 48;
                    for (label, field) in [
                        (format!("Homepage: {}", p.general.homepage), Field::Homepage),
                        (
                            format!("Search URL: {}", p.general.search_engine),
                            Field::Search,
                        ),
                        (
                            format!("Downloads directory: {}", p.downloads.path),
                            Field::Directory,
                        ),
                    ] {
                        widgets.push(Widget::button(
                            (24, line, w - 48, 34),
                            label,
                            Action::Edit(field),
                        ));
                        line += 42;
                    }
                    for (label, n) in [
                        (format!("Restore session: {}", p.general.restore_session), 0),
                        (
                            format!("Always ask where to save: {}", p.downloads.ask_every_time),
                            1,
                        ),
                        (format!("Appearance: {}", p.appearance.theme), 2),
                        (
                            format!("JavaScript default: {}", p.privacy.javascript_enabled),
                            3,
                        ),
                        (
                            format!("Third-party cookies: {}", p.privacy.third_party_cookies),
                            5,
                        ),
                        (
                            format!(
                                "DNS-over-HTTPS via Quad9: {} (restart required)",
                                p.privacy.doh_enabled
                            ),
                            6,
                        ),
                    ] {
                        widgets.push(Widget::button(
                            (24, line, w - 48, 34),
                            label,
                            Action::Toggle(n),
                        ));
                        line += 42;
                    }
                    if let Some(site) = &self.browser.site {
                        widgets.push(Widget::button(
                            (24, line, w - 48, 34),
                            format!("Toggle JavaScript for {site} (reload site to apply)"),
                            Action::Toggle(4),
                        ));
                        line += 42;
                    }
                    widgets.push(Widget::label((24,line,w-48,52),"Privacy defaults: third-party cookies blocked; system DNS; no telemetry. Adblock/list controls are integrated in Phase 10."));
                }
                LocalPage::History | LocalPage::Bookmarks => {
                    widgets.push(Widget::button(
                        (24, y + 48, (w - 48).max(120), 32),
                        format!("Filter: {}", self.browser.filter.text()),
                        Action::Edit(Field::Filter),
                    ));
                    if page == LocalPage::History {
                        widgets.push(Widget::button(
                            (24, y + 88, 110, 30),
                            "Clear all",
                            Action::ClearHistory(0),
                        ));
                        widgets.push(Widget::button(
                            (142, y + 88, 124, 30),
                            "Last hour",
                            Action::ClearHistory(1),
                        ));
                        if self.browser.site.is_some() {
                            widgets.push(Widget::button(
                                (274, y + 88, 138, 30),
                                "Clear this site",
                                Action::ClearHistory(2),
                            ));
                        }
                    } else {
                        widgets.push(Widget::button(
                            (24, y + 88, 160, 30),
                            "Import Netscape HTML",
                            Action::Edit(Field::Import),
                        ));
                        widgets.push(Widget::button(
                            (194, y + 88, 160, 30),
                            "Export Netscape HTML",
                            Action::Export,
                        ));
                    }
                    let query = self.browser.filter.text().to_lowercase();
                    let entries: Vec<(String, String)> = if page == LocalPage::History {
                        self.profile
                            .data
                            .history
                            .iter()
                            .rev()
                            .map(|v| (v.url.to_string(), v.title.clone()))
                            .collect()
                    } else {
                        self.profile
                            .data
                            .bookmarks
                            .iter()
                            .map(|v| (v.url.to_string(), v.title.clone()))
                            .collect()
                    };
                    let rows = ((self.viewport_h() as i32 - 210).max(46) / 46) as usize;
                    for (row, (url, title)) in entries
                        .into_iter()
                        .filter(|(u, t)| {
                            u.to_lowercase().contains(&query) || t.to_lowercase().contains(&query)
                        })
                        .skip(self.browser.offset)
                        .take(rows)
                        .enumerate()
                    {
                        let yy = y + 130 + row as i32 * 46;
                        widgets.push(Widget::button(
                            (24, yy, w - 120, 42),
                            format!("{title}\n{url}"),
                            Action::Navigate(url.clone()),
                        ));
                        if page == LocalPage::Bookmarks {
                            widgets.push(Widget::button(
                                (w - 88, yy, 64, 42),
                                "Remove",
                                Action::RemoveBookmark(url),
                            ));
                        }
                    }
                    widgets.push(Widget::button(
                        (24, h - STATUS_H - 42, 95, 30),
                        "Previous",
                        Action::Next(-1),
                    ));
                    widgets.push(Widget::button(
                        (130, h - STATUS_H - 42, 95, 30),
                        "Next",
                        Action::Next(1),
                    ));
                }
                LocalPage::Downloads => {
                    widgets.push(Widget::label((24,y+42,w-48,40),"Five transfers at most. Data streams to disk; pause/resume uses validated Range/If-Range. Files never open automatically."));
                    let rows = ((self.viewport_h() as i32 - 160).max(80) / 92) as usize;
                    for (row, d) in self
                        .profile
                        .data
                        .downloads
                        .iter()
                        .rev()
                        .skip(self.browser.offset)
                        .take(rows)
                        .enumerate()
                    {
                        let yy = y + 88 + row as i32 * 92;
                        let (rate, eta) =
                            self.browser.rates.get(&d.id).copied().unwrap_or_default();
                        let total = d
                            .total
                            .map_or_else(|| "unknown".into(), |n| format_bytes(n as usize));
                        widgets.push(Widget::label(
                            (24, yy, w - 48, 50),
                            format!(
                                "{} — {:?}\n{} / {} · {}/s · ETA {} · {}",
                                d.destination
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy(),
                                d.state,
                                format_bytes(d.received as usize),
                                total,
                                format_bytes(rate as usize),
                                eta.map_or_else(|| "—".into(), |n| format!("{n}s")),
                                d.message
                            ),
                        ));
                        let action = if matches!(
                            d.state,
                            profile::DownloadState::Running | profile::DownloadState::Queued
                        ) {
                            Action::Pause(d.id)
                        } else {
                            Action::Resume(d.id)
                        };
                        if !matches!(
                            d.state,
                            profile::DownloadState::Complete | profile::DownloadState::Cancelled
                        ) {
                            widgets.push(Widget::button(
                                (24, yy + 54, 92, 28),
                                if matches!(action, Action::Pause(_)) {
                                    "Pause"
                                } else {
                                    "Resume"
                                },
                                action,
                            ));
                            widgets.push(Widget::button(
                                (124, yy + 54, 92, 28),
                                "Cancel",
                                Action::Cancel(d.id),
                            ));
                        }
                        widgets.push(Widget::button(
                            (224, yy + 54, 130, 28),
                            "Open folder",
                            Action::Folder(d.id),
                        ));
                    }
                    widgets.push(Widget::button(
                        (24, h - STATUS_H - 42, 95, 30),
                        "Previous",
                        Action::Next(-1),
                    ));
                    widgets.push(Widget::button(
                        (130, h - STATUS_H - 42, 95, 30),
                        "Next",
                        Action::Next(1),
                    ));
                }
                LocalPage::Devtools => {
                    for (i, label) in ["Elements", "Console", "Network"].iter().enumerate() {
                        widgets.push(Widget::button(
                            (24 + i as i32 * 130, y + 44, 120, 30),
                            *label,
                            Action::Devtools(i as u8),
                        ));
                    }
                    let rows = ((self.viewport_h() as i32 - 140).max(25) / 25) as usize;
                    let lines:Vec<String>=match self.browser.devtools{
                        0=>self.browser.inspection.as_ref().map_or_else(||vec!["Open F12 from a web document to inspect its DOM and computed styles".into()],|inspect|{
                            inspect.document.elements_from(0).into_iter().take(2000).filter_map(|node|{let html::NodeData::Element(element)=&inspect.document.get(node).data else{return None;};let style=inspect.styles.get(node);Some(format!("<{} id={:?} class={:?}> display={:?} color={:?}",element.tag_name,inspect.document.get_attribute(node,"id"),inspect.document.get_attribute(node,"class"),style.map(|s|s.display),style.map(|s|s.color)))}).collect()
                        }),
                        1=>self.browser.inspection.as_ref().map_or_else(Vec::new,|inspect|inspect.console.clone()),
                        _=>self.shared.client.request_log().iter().rev().map(|r|format!("{} {} · {:?} · {} bytes · {}",r.method,r.url,r.status,r.received,r.state)).collect()
                    };
                    for (i, line) in lines
                        .into_iter()
                        .skip(self.browser.offset)
                        .take(rows)
                        .enumerate()
                    {
                        widgets.push(Widget::label(
                            (24, y + 86 + i as i32 * 25, w - 48, 25),
                            line,
                        ));
                    }
                    widgets.push(Widget::button(
                        (24, h - STATUS_H - 42, 95, 30),
                        "Previous",
                        Action::Next(-1),
                    ));
                    widgets.push(Widget::button(
                        (130, h - STATUS_H - 42, 95, 30),
                        "Next",
                        Action::Next(1),
                    ));
                }
            }
        }
        if self.browser.find_open {
            let y = h - STATUS_H - FIND_H;
            widgets.push(Widget::button(
                (8, y + 4, (w - 230).clamp(100, 420), FIND_H - 8),
                format!("Find: {}", self.browser.find_query.text()),
                Action::FindFocus,
            ));
            widgets.push(Widget::label(
                (w - 220, y + 4, 110, FIND_H - 8),
                format!(
                    "{} / {}",
                    if self.browser.find_matches.is_empty() {
                        0
                    } else {
                        self.browser.find_active + 1
                    },
                    self.browser.find_matches.len()
                ),
            ));
            widgets.push(Widget::button(
                (w - 108, y + 4, 30, FIND_H - 8),
                "↑",
                Action::FindNext(-1),
            ));
            widgets.push(Widget::button(
                (w - 72, y + 4, 30, FIND_H - 8),
                "↓",
                Action::FindNext(1),
            ));
            widgets.push(Widget::button(
                (w - 36, y + 4, 30, FIND_H - 8),
                "×",
                Action::FindClose,
            ));
        }
        if self.omnibox_focused && !self.omnibox.text().is_empty() {
            let rect = self.toolbar_layout().omnibox_rect;
            for (i, (url, title)) in self
                .profile
                .data
                .suggestions(self.omnibox.text(), 6)
                .into_iter()
                .enumerate()
            {
                let mut widget = Widget::button(
                    (rect.0, rect.1 + rect.3 + i as i32 * 32, rect.2, 32),
                    format!("{title} · {url}"),
                    Action::Suggest(url),
                );
                widget.overlay = true;
                widget.selected = self.browser.suggestion == Some(i);
                widgets.push(widget);
            }
        }
        if self.browser.menu {
            for (i, (label, action)) in [
                ("New tab · Ctrl+T", Action::New),
                ("Duplicate tab", Action::Duplicate),
                ("History · Ctrl+H", Action::Local(LocalPage::History)),
                ("Bookmarks · Ctrl+B", Action::Local(LocalPage::Bookmarks)),
                ("Downloads · Ctrl+J", Action::Local(LocalPage::Downloads)),
                ("Preferences", Action::Local(LocalPage::Preferences)),
                ("Save as · Ctrl+S", Action::Save),
                ("Print PDF · Ctrl+P", Action::Print),
                ("Developer tools · F12", Action::Local(LocalPage::Devtools)),
            ]
            .into_iter()
            .enumerate()
            {
                let mut widget = Widget::button(
                    (w - 280, TABS_H + NAV_H + i as i32 * 34, 270, 34),
                    label,
                    action,
                );
                widget.overlay = true;
                widgets.push(widget);
            }
        }
        if let Some(field) = &self.browser.editor {
            let width = (w - 40).clamp(200, 800);
            let x = (w - width) / 2;
            let yy = (TOOLBAR_H + 90).min(h - 180);
            let title = if *field == Field::Save {
                "Save to a local file (duplicates receive a suffix)"
            } else if *field == Field::Import {
                "Import bookmarks from a local HTML file"
            } else {
                "Edit value · Enter saves · Escape cancels"
            };
            for (rect, label, action) in [
                ((x, yy, width, 36), title.to_owned(), None),
                (
                    (x, yy + 36, width, 42),
                    self.browser.edit.text().to_owned(),
                    Some(Action::Edit(field.clone())),
                ),
                (
                    (x, yy + 82, width / 2 - 4, 34),
                    "Save / Apply".into(),
                    Some(Action::Commit),
                ),
                (
                    (x + width / 2 + 4, yy + 82, width / 2 - 4, 34),
                    "Cancel".into(),
                    Some(Action::Dismiss),
                ),
            ] {
                let mut widget = Widget::label(rect, label);
                widget.action = action;
                widget.overlay = true;
                widgets.push(widget);
            }
        }
        for (i, widget) in widgets.iter_mut().enumerate() {
            widget.selected |= self.browser.focused == Some(i);
        }
        if self.local_page == Some(LocalPage::Preferences) {
            let bottom = TOOLBAR_H + self.viewport_h() as i32;
            widgets.retain_mut(|widget| {
                if widget.rect.1 >= TOOLBAR_H
                    && !widget.overlay
                    && widget.rect.1 != h - STATUS_H - FIND_H + 4
                {
                    widget.rect.1 -= self.scroll_y as i32;
                    widget.rect.1 >= TOOLBAR_H && widget.rect.1 + widget.rect.3 <= bottom
                } else {
                    true
                }
            });
        }
        widgets
    }
    fn open_devtools(&mut self) {
        if self.local_page != Some(LocalPage::Devtools) {
            if let Some(page) = &self.page {
                self.browser.inspection = Some(Inspection {
                    document: page.document.clone(),
                    styles: self.styles.clone(),
                    console: self.script_console.clone(),
                    source: self.active_tab,
                });
            }
            if let Err(error) = self.new_tab(None) {
                self.browser.notice = error;
                return;
            }
            self.open_local(LocalPage::Devtools);
        } else if let Some(source) = self.browser.inspection.as_ref().map(|i| i.source) {
            let visible = self.active_tab;
            self.swap_context(source);
            if let Some(page) = &self.page {
                self.browser.inspection = Some(Inspection {
                    document: page.document.clone(),
                    styles: self.styles.clone(),
                    console: self.script_console.clone(),
                    source,
                });
            }
            self.swap_context(visible);
            self.request_redraw();
        }
    }
    pub(super) fn handle_browser_key(&mut self, event: &KeyEvent) -> bool {
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        if ctrl && !self.modifiers.alt_key() {
            let action = match &event.logical_key {
                Key::Character(ch) => match ch.to_ascii_lowercase().as_str() {
                    "t" => Some(ChromeAction::New),
                    "w" => Some(ChromeAction::Close(self.active_tab)),
                    "d" => Some(ChromeAction::Bookmark),
                    "h" => Some(ChromeAction::Local(LocalPage::History)),
                    "b" => Some(ChromeAction::Local(LocalPage::Bookmarks)),
                    "j" => Some(ChromeAction::Local(LocalPage::Downloads)),
                    "," => Some(ChromeAction::Local(LocalPage::Preferences)),
                    "s" => Some(ChromeAction::Save),
                    "p" => Some(ChromeAction::Print),
                    "f" => {
                        self.browser.find_open = true;
                        self.browser.find_focused = true;
                        self.omnibox_focused = false;
                        self.browser.find_query.select_all();
                        self.content_measured_for = None;
                        self.request_find();
                        self.request_redraw();
                        return true;
                    }
                    "v" if self.omnibox_focused
                        || self.browser.editor.is_some()
                        || self.browser.find_focused =>
                    {
                        self.paste_chrome();
                        return true;
                    }
                    "a" if self.browser.editor.is_some() => {
                        self.browser.edit.select_all();
                        self.request_redraw();
                        return true;
                    }
                    "a" if self.browser.find_focused => {
                        self.browser.find_query.select_all();
                        self.request_redraw();
                        return true;
                    }
                    "a" if self.omnibox_focused => {
                        self.omnibox.select_all();
                        self.request_redraw();
                        return true;
                    }
                    "c" | "x"
                        if self.omnibox_focused
                            || self.browser.editor.is_some()
                            || self.browser.find_focused =>
                    {
                        let edit = if self.browser.editor.is_some() {
                            &mut self.browser.edit
                        } else if self.browser.find_focused {
                            &mut self.browser.find_query
                        } else {
                            &mut self.omnibox
                        };
                        let text = edit.text().to_owned();
                        if ch.eq_ignore_ascii_case("x") {
                            edit.select_all();
                            edit.delete();
                        }
                        let _ = crate::web_ui::spawn_clipboard(Some(text), |_| {});
                        self.request_redraw();
                        return true;
                    }
                    "r" if shift => {
                        self.reload_mode(net::navigation::CacheMode::Bypass);
                        return true;
                    }
                    "+" | "=" => {
                        self.zoom_by(1.1);
                        return true;
                    }
                    "-" => {
                        self.zoom_by(1.0 / 1.1);
                        return true;
                    }
                    "0" => {
                        self.zoom_by(1.0 / self.zoom);
                        return true;
                    }
                    n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() => {
                        let index = n.parse::<usize>().unwrap_or(1);
                        self.select_tab(if index == 9 {
                            self.tabs.len().saturating_sub(1)
                        } else {
                            index.saturating_sub(1)
                        });
                        return true;
                    }
                    _ => None,
                },
                Key::Named(NamedKey::Tab) => {
                    let n = self.tabs.len();
                    if n > 0 {
                        self.select_tab((self.active_tab + if shift { n - 1 } else { 1 }) % n);
                    }
                    return true;
                }
                Key::Named(NamedKey::PageUp | NamedKey::PageDown) => {
                    let n = self.tabs.len();
                    if n > 0 {
                        let next = if event.logical_key == Key::Named(NamedKey::PageUp) {
                            self.active_tab.saturating_sub(1)
                        } else {
                            (self.active_tab + 1).min(n - 1)
                        };
                        if shift {
                            self.reorder_tab(self.active_tab, next);
                        } else {
                            self.select_tab(next);
                        }
                    }
                    return true;
                }
                _ => None,
            };
            if let Some(action) = action {
                if !event.repeat {
                    self.chrome_action(action);
                }
                return true;
            }
        }
        if event.logical_key == Key::Named(NamedKey::F12) {
            self.open_devtools();
            return true;
        }
        if event.logical_key == Key::Named(NamedKey::F10) {
            self.chrome_action(Action::Menu);
            return true;
        }
        if event.logical_key == Key::Named(NamedKey::F3) {
            self.find_next(if shift { -1 } else { 1 });
            return true;
        }
        if self.web_dialog.is_some() {
            return false;
        }
        if self.browser.editor.is_some() {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => self.chrome_action(Action::Dismiss),
                Key::Named(NamedKey::Enter) => self.chrome_action(Action::Commit),
                _ => {
                    edit_key(&mut self.browser.edit, event, 8192);
                    self.request_redraw();
                }
            }
            return true;
        }
        if self.browser.find_focused {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => self.chrome_action(Action::FindClose),
                Key::Named(NamedKey::Enter) => self.find_next(if shift { -1 } else { 1 }),
                _ => {
                    edit_key(&mut self.browser.find_query, event, 512);
                    self.request_find();
                    self.request_redraw();
                }
            }
            return true;
        }
        if self.local_page.is_some() || self.browser.menu || self.browser.focused.is_some() {
            match &event.logical_key {
                Key::Named(NamedKey::Tab) => {
                    let focusable: Vec<_> = self
                        .browser
                        .widgets
                        .iter()
                        .enumerate()
                        .filter(|(_, w)| w.action.is_some())
                        .map(|(i, _)| i)
                        .collect();
                    if !focusable.is_empty() {
                        let current = self
                            .browser
                            .focused
                            .and_then(|n| focusable.iter().position(|i| *i == n))
                            .unwrap_or(if shift { 0 } else { focusable.len() - 1 });
                        let next = (current + if shift { focusable.len() - 1 } else { 1 })
                            % focusable.len();
                        self.browser.focused = Some(focusable[next]);
                    }
                    self.request_redraw();
                    return true;
                }
                Key::Named(NamedKey::Enter | NamedKey::Space) => {
                    if let Some(action) = self
                        .browser
                        .focused
                        .and_then(|i| self.browser.widgets.get(i))
                        .and_then(|w| w.action.clone())
                    {
                        self.chrome_action(action);
                        return true;
                    }
                }
                Key::Named(NamedKey::Escape) => {
                    self.browser.menu = false;
                    self.browser.focused = None;
                    self.request_redraw();
                    return true;
                }
                _ => {}
            }
        }
        false
    }
}
fn edit_key(edit: &mut Omnibox, event: &KeyEvent, cap: usize) {
    match &event.logical_key {
        Key::Named(NamedKey::Backspace) => edit.backspace(),
        Key::Named(NamedKey::Delete) => edit.delete(),
        Key::Named(NamedKey::ArrowLeft) => edit.move_caret(-1),
        Key::Named(NamedKey::ArrowRight) => edit.move_caret(1),
        Key::Named(NamedKey::Home) => edit.jump_to(false),
        Key::Named(NamedKey::End) => edit.jump_to(true),
        _ => {
            if let Some(text) = &event.text {
                if !text.chars().any(char::is_control)
                    && (edit.is_selected() || edit.text().len() + text.len() <= cap)
                {
                    edit.insert(text);
                }
            }
        }
    }
}
