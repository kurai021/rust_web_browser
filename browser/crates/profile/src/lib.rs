//! `profile`: Phase 0 stub — spec in `plan/03-general-architecture.md` §3.5.
//!
//! Real on-disk profile in Phase 9 (history, cookies, storage, config).

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
