//! `js`: Phase 0 stub — spec in `plan/08-javascript-engine.md`.
//!
//! Real implementation in Phase 5 (Level 1 interpreter) and Phase 8 (Level 2 + VM).

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
