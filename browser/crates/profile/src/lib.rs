//! Local browser profile: preferences, history, bookmarks, sessions and downloads.
//! Only trusted shell actions reach this crate; it has no network or VM dependency.
mod preferences;
mod writer;
pub use preferences::{Appearance, Downloads, General, Preferences, Privacy, Render};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;
pub use writer::{default_directory, expand_home, Store};

pub const MAX_TABS: usize = 50;
pub const MAX_HISTORY: usize = 10_000;
pub const MAX_BOOKMARKS: usize = 5_000;
pub const MAX_DOWNLOADS: usize = 1_000;
pub const MAX_PROFILE_BYTES: u64 = 16 * 1024 * 1024;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Credentials are never retained in history, autocomplete or session files.
pub fn clean_url(source: &str) -> Option<Url> {
    if source.len() > 8192 {
        return None;
    }
    let mut url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "about") {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Some(url)
}
pub fn short_title(source: &str) -> String {
    source
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Visit {
    pub url: Url,
    pub title: String,
    pub time: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: Url,
    pub title: String,
    pub added: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedTab {
    pub current: String,
    pub back: Vec<String>,
    pub forward: Vec<String>,
    pub scroll_x: f32,
    pub scroll_y: f32,
}
impl Default for SavedTab {
    fn default() -> Self {
        Self {
            current: "about:blank".into(),
            back: Vec::new(),
            forward: Vec::new(),
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    #[default]
    Queued,
    Running,
    Paused,
    Complete,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Download {
    pub id: u64,
    pub url: String,
    pub top: String,
    pub destination: std::path::PathBuf,
    pub partial: std::path::PathBuf,
    pub received: u64,
    pub total: Option<u64>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub mime: String,
    pub state: DownloadState,
    pub message: String,
    pub started: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub history: Vec<Visit>,
    pub bookmarks: Vec<Bookmark>,
    pub session: Session,
    pub zoom: BTreeMap<String, f32>,
    pub downloads: Vec<Download>,
}
impl Data {
    pub fn normalize(&mut self) {
        self.history.retain_mut(|v| {
            let Some(url) = clean_url(v.url.as_str()) else {
                return false;
            };
            v.url = url;
            v.title = short_title(&v.title);
            true
        });
        if self.history.len() > MAX_HISTORY {
            self.history.drain(..self.history.len() - MAX_HISTORY);
        }
        self.bookmarks.retain_mut(|v| {
            let Some(url) = clean_url(v.url.as_str()) else {
                return false;
            };
            v.url = url;
            v.title = short_title(&v.title);
            true
        });
        self.bookmarks.truncate(MAX_BOOKMARKS);
        self.session.tabs.truncate(MAX_TABS);
        for tab in &mut self.session.tabs {
            tab.current =
                clean_url(&tab.current).map_or_else(|| "about:blank".into(), |u| u.to_string());
            for stack in [&mut tab.back, &mut tab.forward] {
                *stack = stack
                    .iter()
                    .filter_map(|u| clean_url(u).map(|u| u.to_string()))
                    .take(200)
                    .collect();
            }
            for scroll in [&mut tab.scroll_x, &mut tab.scroll_y] {
                *scroll = if scroll.is_finite() {
                    scroll.clamp(0.0, 10_000_000.0)
                } else {
                    0.0
                };
            }
        }
        self.session.active = self
            .session
            .active
            .min(self.session.tabs.len().saturating_sub(1));
        self.zoom.retain(|key, value| {
            key.len() <= 2048 && value.is_finite() && (0.8..=2.0).contains(value)
        });
        while self.zoom.len() > 1000 {
            self.zoom.pop_first();
        }
        self.downloads.truncate(MAX_DOWNLOADS);
        for download in &mut self.downloads {
            if matches!(
                download.state,
                DownloadState::Running | DownloadState::Queued
            ) {
                download.state = DownloadState::Paused;
                download.message = "Interrupted; resume explicitly".into();
            }
        }
    }
    pub fn visit(&mut self, url: &Url, title: &str) {
        let Some(url) = clean_url(url.as_str()) else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        if self.history.last().is_some_and(|v| v.url == url) {
            self.history.pop();
        }
        self.history.push(Visit {
            url,
            title: short_title(title),
            time: now(),
        });
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }
    pub fn delete_history(&mut self, since: u64, site: Option<&str>) {
        self.history
            .retain(|v| v.time < since || site.is_some_and(|s| v.url.host_str() != Some(s)));
    }
    pub fn bookmark(&mut self, url: &Url, title: &str) -> bool {
        let Some(url) = clean_url(url.as_str()) else {
            return false;
        };
        if let Some(i) = self.bookmarks.iter().position(|b| b.url == url) {
            self.bookmarks.remove(i);
            return false;
        }
        if self.bookmarks.len() < MAX_BOOKMARKS {
            self.bookmarks.push(Bookmark {
                url,
                title: short_title(title),
                added: now(),
            });
            return true;
        }
        false
    }
    /// Local entries only; opening the omnibox never resolves a host.
    pub fn suggestions(&self, query: &str, limit: usize) -> Vec<(String, String)> {
        let query = query.to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut seen = std::collections::HashSet::new();
        self.bookmarks
            .iter()
            .rev()
            .map(|b| (&b.url, &b.title))
            .chain(self.history.iter().rev().map(|b| (&b.url, &b.title)))
            .filter(|(url, title)| {
                (url.as_str().to_lowercase().contains(&query)
                    || title.to_lowercase().contains(&query))
                    && seen.insert(url.as_str())
            })
            .take(limit.min(20))
            .map(|(url, title)| (url.to_string(), title.clone()))
            .collect()
    }
}
