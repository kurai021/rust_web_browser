//! Per-tab navigation history (plan/10 §10.2.4).
//!
//! Pure back/forward stacks over visited URLs. UI wiring (buttons, keys)
//! lives in `app.rs`; persistence lands in Phase 9.

use url::Url;

/// Back/forward history for one browsing context.
#[derive(Debug, Default, Clone)]
pub struct History {
    back: Vec<Url>,
    current: Option<Url>,
    forward: Vec<Url>,
}

impl History {
    /// Empty history (fresh tab).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently shown URL, if any.
    #[must_use]
    pub fn current(&self) -> Option<&Url> {
        self.current.as_ref()
    }
    pub fn back_target(&self) -> Option<Url> {
        self.back.last().cloned()
    }
    pub fn forward_target(&self) -> Option<Url> {
        self.forward.last().cloned()
    }

    /// Record a new navigation: push current to back, clear forward.
    pub fn navigate(&mut self, url: Url) {
        if let Some(current) = self.current.take() {
            self.back.push(current);
            if self.back.len() > 200 {
                self.back.remove(0);
            }
        }
        self.forward.clear();
        self.current = Some(url);
    }

    /// Go back. Returns the URL to load, if any.
    pub fn go_back(&mut self) -> Option<Url> {
        let prev = self.back.pop()?;
        if let Some(current) = self.current.take() {
            self.forward.push(current);
        }
        self.current = Some(prev.clone());
        Some(prev)
    }

    /// Go forward. Returns the URL to load, if any.
    pub fn go_forward(&mut self) -> Option<Url> {
        let next = self.forward.pop()?;
        if let Some(current) = self.current.take() {
            self.back.push(current);
        }
        self.current = Some(next.clone());
        Some(next)
    }

    /// True when back navigation is possible (button state).
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// True when forward navigation is possible (button state).
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
    pub fn replace(&mut self, url: Url) {
        self.current = Some(url);
    }
    pub fn save(&self) -> profile::SavedTab {
        profile::SavedTab {
            current: self
                .current
                .as_ref()
                .map_or_else(|| "about:blank".into(), |u| u.to_string()),
            back: self
                .back
                .iter()
                .rev()
                .take(200)
                .rev()
                .map(|u| u.to_string())
                .collect(),
            forward: self
                .forward
                .iter()
                .rev()
                .take(200)
                .rev()
                .map(|u| u.to_string())
                .collect(),
            ..Default::default()
        }
    }
    pub fn restore(tab: &profile::SavedTab) -> Self {
        Self {
            back: tab
                .back
                .iter()
                .filter_map(|u| profile::clean_url(u))
                .take(200)
                .collect(),
            current: profile::clean_url(&tab.current),
            forward: tab
                .forward
                .iter()
                .filter_map(|u| profile::clean_url(u))
                .take(200)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(path: &str) -> Url {
        format!("https://example.com{path}").parse().unwrap()
    }

    #[test]
    fn empty_history_has_no_current() {
        let history = History::new();
        assert_eq!(history.current(), None);
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn navigate_and_walk_back_forward() {
        let mut history = History::new();
        history.navigate(url("/a"));
        history.navigate(url("/b"));
        assert_eq!(history.current(), Some(&url("/b")));
        assert_eq!(history.go_back(), Some(url("/a")));
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());
        assert_eq!(history.go_forward(), Some(url("/b")));
    }

    #[test]
    fn new_navigation_clears_forward() {
        let mut history = History::new();
        history.navigate(url("/a"));
        history.navigate(url("/b"));
        history.go_back();
        history.navigate(url("/c"));
        assert!(!history.can_go_forward());
        assert_eq!(history.current(), Some(&url("/c")));
    }
}
