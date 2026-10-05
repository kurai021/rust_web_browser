//! Omnibox editing state (plan/10 §10.2.3, plan/04 §4.3.1).
//!
//! Pure single-line buffer: caret movement, insertion, deletion.
//! Rendering and `winit` key handling live in `app.rs`.

/// Editable single-line text with a caret (byte index into `text`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Omnibox {
    text: String,
    /// Caret byte offset; always a char boundary.
    caret: usize,
    /// Whole content selected (typing replaces it).
    selected: bool,
}

impl Omnibox {
    /// Empty box.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Caret byte offset.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Replace the whole content (e.g. show the current page URL).
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.caret = self.text.len();
        self.selected = false;
    }

    /// Select the whole content (focus semantics: typing replaces it).
    pub fn select_all(&mut self) {
        self.caret = self.text.len();
        self.selected = !self.text.is_empty();
    }

    /// Whether the whole content is selected.
    #[must_use]
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// Insert text, replacing a selection first.
    pub fn insert(&mut self, snippet: &str) {
        if self.selected {
            self.text.clear();
            self.caret = 0;
            self.selected = false;
        }
        self.text.insert_str(self.caret, snippet);
        self.caret += snippet.len();
    }

    /// Delete the char before the caret (or the selection).
    pub fn backspace(&mut self) {
        if self.selected {
            self.text.clear();
            self.caret = 0;
            self.selected = false;
            return;
        }
        let Some(prev_len) = self.char_before_len() else {
            return;
        };
        self.caret -= prev_len;
        self.text.remove(self.caret);
    }

    /// Delete the char under the caret (or the selection).
    pub fn delete(&mut self) {
        if self.selected {
            self.text.clear();
            self.caret = 0;
            self.selected = false;
            return;
        }
        if self.caret < self.text.len() {
            self.text.remove(self.caret);
        }
    }

    /// Move the caret by whole chars (`+1` right, `-1` left), clamped.
    /// Any move collapses the selection.
    pub fn move_caret(&mut self, delta: i32) {
        self.selected = false;
        if delta > 0 {
            for _ in 0..delta {
                match self.text[self.caret..].chars().next() {
                    Some(ch) => self.caret += ch.len_utf8(),
                    None => break,
                }
            }
        } else {
            for _ in 0..-delta {
                match self.char_before_len() {
                    Some(len) => self.caret -= len,
                    None => break,
                }
            }
        }
    }

    /// Jump to either end (collapses the selection).
    pub fn jump_to(&mut self, end: bool) {
        self.selected = false;
        self.caret = if end { self.text.len() } else { 0 };
    }

    fn char_before_len(&self) -> Option<usize> {
        self.text[..self.caret]
            .chars()
            .next_back()
            .map(|ch| ch.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace() {
        let mut box_ = Omnibox::new();
        box_.insert("example.com");
        assert_eq!(box_.text(), "example.com");
        box_.backspace();
        assert_eq!(box_.text(), "example.co");
    }

    #[test]
    fn select_all_replaces_on_type() {
        let mut box_ = Omnibox::new();
        box_.insert("example.com");
        box_.select_all();
        assert!(box_.is_selected());
        box_.insert("other.org");
        assert_eq!(box_.text(), "other.org");
        assert!(!box_.is_selected());
    }

    #[test]
    fn caret_moves_by_chars_not_bytes() {
        let mut box_ = Omnibox::new();
        box_.insert("añ");
        assert_eq!(box_.caret(), 3);
        box_.move_caret(-1);
        assert_eq!(box_.caret(), 1);
        box_.delete();
        assert_eq!(box_.text(), "a");
    }

    #[test]
    fn set_text_parks_caret_at_end() {
        let mut box_ = Omnibox::new();
        box_.set_text("https://example.com/");
        assert_eq!(box_.caret(), "https://example.com/".len());
    }
}
