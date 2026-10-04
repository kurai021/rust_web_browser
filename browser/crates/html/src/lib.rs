//! `html`: Phase 0 stub — spec in `plan/05-html-parser.md`.
//!
//! Real implementation in Phase 2 (tokenizer + tree builder + DOM).

/// Stub marker: always `true` in Phase 0.
#[must_use]
pub fn stub() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_active() {
        assert!(stub());
    }
}
