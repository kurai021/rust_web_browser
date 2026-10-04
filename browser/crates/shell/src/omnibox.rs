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
    }

    /// Insert text at the caret.
    pub fn insert(&mut self, snippet: &str) {
        self.text.insert_str(self.caret, snippet);
        self.caret += snippet.len();
    }

    /// Delete the char before the caret.
    pub fn backspace(&mut self) {
        let Some(prev_len) = self.char_before_len() else {
            return;
        };
        self.caret -= prev_len;
        self.text.remove(self.caret);
    }

    /// Delete the char under the caret.
    pub fn delete(&mut self) {
        if self.caret < self.text.len() {
            self.text.remove(self.caret);
        }
    }

    /// Move the caret by whole chars (`+1` right, `-1` left), clamped.
    pub fn move_caret(&mut self, delta: i32) {
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

    /// Jump to either end.
    pub fn jump_to(&mut self, end: bool) {
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
