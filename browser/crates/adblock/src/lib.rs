//! `adblock`: Phase 0 stub — spec in `plan/11-downloads-adblock-privacy.md` §11.2.
//!
//! Real filter engine in Phase 10 (EasyList/EasyPrivacy, K2 matcher per doc 15).

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
