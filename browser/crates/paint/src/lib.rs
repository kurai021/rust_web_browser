//! `paint`: Phase 0 stub — spec in `plan/07-layout-render-paint.md` §7.4–7.5.
//!
//! Real implementation in Phase 4 (display list, wgpu + software fallback).

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
