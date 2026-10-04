//! `layout`: Phase 0 stub — spec in `plan/07-layout-render-paint.md` §7.2–7.3.
//!
//! Real implementation in Phase 4 (flow) and Phase 6 (flex/grid/position).

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
