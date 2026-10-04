//! `dom_bindings`: Phase 0 stub — spec in `plan/09-dom-bom-webapis.md`.
//!
//! Real JS↔DOM glue in Phase 5 (minimal) and Phase 7 (complete).

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
