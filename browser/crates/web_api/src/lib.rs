//! `web_api`: Phase 0 stub — spec in `plan/09-dom-bom-webapis.md` §9.4.
//!
//! Real Web APIs in Phase 7 (fetch/XHR, storage, observers, Canvas2D subset).

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
