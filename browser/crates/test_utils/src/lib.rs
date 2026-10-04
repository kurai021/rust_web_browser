//! `test_utils`: Phase 0 stub — spec in `plan/13-testing-compatibility.md`.
//!
//! Real WPT/golden/fuzz harness from Phase 2 onward.

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
