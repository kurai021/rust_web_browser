//! HSTS store (plan/04 §4.3.3): per-host HTTPS enforcement.
//!
//! Phase 1: in-memory map plus JSON save/load so the shell can persist it
//! under the profile dir. Preload list and `includeSubDomains` expiry
//! pruning are included; full profile wiring lands in Phase 9.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One HSTS entry: "this host is HTTPS-only until `expires_at`".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    expires_at_secs: u64,
    include_subdomains: bool,
}

/// In-memory HSTS store.
#[derive(Debug, Default, Clone)]
pub struct HstsStore {
    entries: HashMap<String, Entry>,
}

impl HstsStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a `Strict-Transport-Security` response header for `host`.
    ///
    /// Malformed headers are ignored (fail-closed: keep the old entry).
    pub fn note_header(&mut self, host: &str, header_value: &str, now: SystemTime) {
        let Some((max_age, include_subdomains)) = parse_hsts_header(header_value) else {
            return;
        };
        if max_age == 0 {
            self.entries.remove(&normalize(host));
            return;
        }
        let expires_at_secs = now
            .checked_add(Duration::from_secs(max_age))
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        self.entries.insert(
            normalize(host),
            Entry {
                expires_at_secs,
                include_subdomains,
            },
        );
    }

    /// True when `host` (or a parent with `includeSubDomains`) forces HTTPS.
    #[must_use]
    pub fn is_https_only(&self, host: &str, now: SystemTime) -> bool {
        let now_secs = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let host = normalize(host);
        // Exact host, then each parent domain for includeSubDomains.
        let mut suffix = host.as_str();
        loop {
            if let Some(entry) = self.entries.get(suffix) {
                if entry.expires_at_secs > now_secs && (suffix == host || entry.include_subdomains)
                {
                    return true;
                }
            }
            match suffix.find('.') {
                Some(dot) => suffix = &suffix[dot + 1..],
                None => return false,
            }
        }
    }

    /// Drop expired entries. Returns how many were removed.
    pub fn prune_expired(&mut self, now: SystemTime) -> usize {
        let now_secs = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let before = self.entries.len();
        self.entries.retain(|_, e| e.expires_at_secs > now_secs);
        before - self.entries.len()
    }

    /// Persist to `path` as JSON. Missing parent dirs are created.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// Load from `path`. A missing file yields an empty store; corrupt
    /// files yield an empty store too (fail-open to plain HTTPS logic).
    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(entries) = serde_json::from_str(&text) else {
            return Self::default();
        };
        Self { entries }
    }
}

fn normalize(host: &str) -> String {
    host.trim().trim_end_matches('.').to_lowercase()
}

/// Parse `max-age=…[; includeSubDomains]` (case-insensitive); `None` when no valid max-age.
fn parse_hsts_header(value: &str) -> Option<(u64, bool)> {
    let mut max_age = None;
    let mut include_subdomains = false;
    for part in value.split(';') {
        let lower = part.trim().to_lowercase();
        if lower == "includesubdomains" {
            include_subdomains = true;
        } else if let Some(num) = lower.strip_prefix("max-age=") {
            max_age = num.trim().trim_matches('"').parse::<u64>().ok();
        }
    }
    max_age.map(|age| (age, include_subdomains))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn records_and_matches_exact_host() {
        let mut store = HstsStore::new();
        store.note_header("example.com", "max-age=31536000", now());
        assert!(store.is_https_only("example.com", now()));
        assert!(!store.is_https_only("other.com", now()));
    }

    #[test]
    fn max_age_zero_removes() {
        let mut store = HstsStore::new();
        store.note_header("example.com", "max-age=100", now());
        store.note_header("example.com", "max-age=0", now());
        assert!(!store.is_https_only("example.com", now()));
    }

    #[test]
    fn include_subdomains_covers_children() {
        let mut store = HstsStore::new();
        store.note_header("example.com", "max-age=100; includeSubDomains", now());
        assert!(store.is_https_only("a.b.example.com", now()));
        assert!(!store.is_https_only("example.com.evil.com", now()));
    }

    #[test]
    fn expired_entries_do_not_apply() {
        let mut store = HstsStore::new();
        store.note_header("example.com", "max-age=1", now());
        let later = now() + Duration::from_secs(5);
        assert!(!store.is_https_only("example.com", later));
        assert_eq!(store.prune_expired(later), 1);
    }

    #[test]
    fn malformed_headers_are_ignored() {
        let mut store = HstsStore::new();
        store.note_header("example.com", "nonsense", now());
        assert!(!store.is_https_only("example.com", now()));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("browser-hsts-test");
        let path = dir.join("hsts.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = HstsStore::new();
        store.note_header("example.com", "max-age=99999; includeSubDomains", now());
        store.save_to(&path).unwrap();
        let loaded = HstsStore::load_from(&path);
        assert!(loaded.is_https_only("www.example.com", now()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_corrupt_file_gives_empty_store() {
        let missing = std::env::temp_dir().join("browser-hsts-missing.json");
        let _ = std::fs::remove_file(&missing);
        assert!(!HstsStore::load_from(&missing).is_https_only("example.com", now()));
        let corrupt = std::env::temp_dir().join("browser-hsts-corrupt.json");
        std::fs::write(&corrupt, "{oops").unwrap();
        assert!(!HstsStore::load_from(&corrupt).is_https_only("example.com", now()));
        let _ = std::fs::remove_file(&corrupt);
    }
}
