//! Owned cookie policy over the auxiliary cookie/public-suffix parsers.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use url::Url;
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Stored {
    name: String,
    value: String,
    domain: String,
    path: String,
    host_only: bool,
    secure: bool,
    http_only: bool,
    same_site: String,
    expires: Option<i64>,
    created: u64,
}
#[derive(Default)]
pub struct CookieJar {
    entries: Mutex<Vec<Stored>>,
    persistence: Mutex<()>,
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
pub fn site(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    format!(
        "{}://{}",
        url.scheme(),
        psl::domain_str(host).unwrap_or(host)
    )
}
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}
fn path_matches(path: &str, prefix: &str) -> bool {
    path == prefix
        || path.starts_with(prefix)
            && (prefix.ends_with('/') || path.as_bytes().get(prefix.len()) == Some(&b'/'))
}
impl CookieJar {
    pub fn header(&self, url: &Url, top: &Url, document: bool) -> String {
        // Third-party cookies are blocked by default. No unpartitioned fallback.
        if site(url) != site(top) {
            return String::new();
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let time = now();
        entries.retain(|c| c.expires.is_none_or(|e| e > time));
        let host = url.host_str().unwrap_or("");
        let mut values = entries
            .iter()
            .filter(|c| {
                (if c.host_only {
                    host == c.domain
                } else {
                    domain_matches(host, &c.domain)
                }) && path_matches(url.path(), &c.path)
                    && (!c.secure || url.scheme() == "https")
                    && (!document || !c.http_only)
            })
            .collect::<Vec<_>>();
        values.sort_by_key(|c| (std::cmp::Reverse(c.path.len()), c.created));
        values
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }
    pub fn set(&self, url: &Url, top: &Url, source: &str, document: bool) -> bool {
        if !matches!(url.scheme(), "http" | "https")
            || source.len() > 8192
            || site(url) != site(top)
        {
            return false;
        }
        let Ok(cookie) = cookie::Cookie::parse(source.to_owned()) else {
            return false;
        };
        if cookie.name().is_empty()
            || cookie
                .name()
                .chars()
                .any(|c| c.is_control() || ";=, ".contains(c))
        {
            return false;
        }
        if cookie
            .value()
            .bytes()
            .any(|b| b < 0x20 || b == 0x7f || b == b';')
        {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        let domain = cookie
            .domain()
            .unwrap_or(&host)
            .trim_start_matches('.')
            .to_ascii_lowercase();
        let host_only = cookie.domain().is_none();
        if !domain_matches(&host, &domain)
            || (!host_only
                && (url
                    .host()
                    .is_some_and(|h| matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)))
                    || psl::suffix_str(&domain) == Some(domain.as_str())))
        {
            return false;
        }
        let secure = cookie.secure().unwrap_or(false);
        let http_only = cookie.http_only().unwrap_or(false);
        if secure && url.scheme() != "https" || document && http_only {
            return false;
        }
        let same_site = match cookie.same_site() {
            Some(cookie::SameSite::Strict) => "Strict",
            Some(cookie::SameSite::None) => "None",
            _ => "Lax",
        }
        .to_owned();
        if same_site == "None" && !secure {
            return false;
        }
        let default_path =
            url.path()
                .rsplit_once('/')
                .map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
        let path = cookie
            .path()
            .filter(|p| p.starts_with('/'))
            .unwrap_or(default_path)
            .to_owned();
        if cookie.name().starts_with("__Secure-") && !secure
            || cookie.name().starts_with("__Host-") && (!secure || !host_only || path != "/")
        {
            return false;
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let old = entries
            .iter()
            .find(|c| c.domain == domain && c.path == path && c.name == cookie.name());
        if document && old.is_some_and(|c| c.http_only) {
            return false;
        }
        if url.scheme() != "https"
            && entries.iter().any(|c| {
                c.secure
                    && c.name == cookie.name()
                    && domain_matches(&domain, &c.domain)
                    && path_matches(&path, &c.path)
            })
        {
            return false;
        }
        let created = old.map_or(
            entries
                .iter()
                .map(|c| c.created)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            |c| c.created,
        );
        entries.retain(|c| !(c.domain == domain && c.path == path && c.name == cookie.name()));
        let expires = cookie
            .max_age()
            .map(|d| now().saturating_add(d.whole_seconds()))
            .or_else(|| cookie.expires_datetime().map(|d| d.unix_timestamp()));
        if expires.is_some_and(|e| e <= now()) {
            return true;
        }
        if entries.len() >= 4096 || entries.iter().filter(|c| c.domain == domain).count() >= 180 {
            return false;
        }
        entries.push(Stored {
            name: cookie.name().into(),
            value: cookie.value().into(),
            domain,
            path,
            host_only,
            secure,
            http_only,
            same_site,
            expires,
            created,
        });
        true
    }
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let _guard = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
        let data = serde_json::to_vec(
            &self
                .entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|c| c.expires.is_some_and(|e| e > now()))
                .collect::<Vec<_>>(),
        )?;
        if data.len() > 8 * 1024 * 1024 {
            return Err(std::io::Error::other("cookie quota"));
        }
        let tmp = path.with_extension("tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        std::io::Write::write_all(&mut file, &data)?;
        std::fs::rename(tmp, path)
    }
    pub fn load(&self, path: &std::path::Path) -> std::io::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        if path.metadata()?.len() > 8 * 1024 * 1024 {
            return Err(std::io::Error::other("cookie file quota"));
        }
        let mut entries: Vec<Stored> = serde_json::from_slice(&std::fs::read(path)?)?;
        entries.retain(|c| c.expires.is_some_and(|e| e > now()));
        entries.truncate(4096);
        *self.entries.lock().unwrap_or_else(|e| e.into_inner()) = entries;
        Ok(())
    }
}
