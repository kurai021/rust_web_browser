//! `css`: Phase 0 stub — spec in `plan/06-css.md`.
//!
//! Real implementation in Phase 3 (syntax, selectors, cascade, values).

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
