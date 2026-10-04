//! URL parsing and omnibox classification (plan/04 §4.3.1).
//!
//! WHATWG URLs via the `url` crate. `file://` is accepted for local tests
//! only and never treated as a navigable web scheme.

use url::Url;

use crate::Error;

/// What the user typed in the omnibox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserInput {
    /// Navigate to this URL.
    Url(Url),
    /// Run a search for this query (search engine wiring lands in Phase 9).
    Search(String),
}

/// Parse an absolute URL, rejecting non-http(s) schemes for navigation.
///
/// `file://` URLs parse fine but are reported as [`Error::UnsupportedScheme`]
/// so callers can allow them in tests while the navigator refuses them.
pub fn parse_url(text: &str) -> Result<Url, Error> {
    let trimmed = text.trim();
    let url = Url::parse(trimmed).map_err(|_| Error::InvalidUrl(trimmed.to_owned()))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(Error::UnsupportedScheme(other.to_owned())),
    }
}

/// Decide whether omnibox text is a URL or a search query.
///
/// Rule (plan/04 §4.3.1): navigates when the text parses as a URL whose host
/// is dotted, `localhost`, or an IP literal. Everything else is a search.
/// A bare `example.com` (no scheme) is upgraded to `https://example.com`.
#[must_use]
pub fn classify_user_input(text: &str) -> UserInput {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return UserInput::Search(String::new());
    }
    // Fast path: absolute URL with a usable host.
    if let Ok(url) = Url::parse(trimmed) {
        if matches!(url.scheme(), "http" | "https") && navigable_host(&url) {
            return UserInput::Url(url);
        }
        if url.scheme() == "file" {
            return UserInput::Search(trimmed.to_owned());
        }
        return UserInput::Search(trimmed.to_owned());
    }
    // Bare host? Try https:// upgrade (plan/04 §4.3.3).
    if looks_like_host(trimmed) {
        if let Ok(url) = Url::parse(&format!("https://{trimmed}")) {
            return UserInput::Url(url);
        }
    }
    UserInput::Search(trimmed.to_owned())
}

/// True for hosts worth navigating to: dotted names, localhost, IPs.
fn navigable_host(url: &Url) -> bool {
    match url.host_str() {
        None => false,
        Some("localhost") => true,
        Some(host) => host.contains('.') || is_ip_literal(host),
    }
}

/// True when the text looks like `host[:port][/path...]` without a scheme.
fn looks_like_host(text: &str) -> bool {
    if text.contains(' ') || text.contains('\\') {
        return false;
    }
    let authority = text.split('/').next().unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    host == "localhost" || host.contains('.') || is_ip_literal(host)
}

fn is_ip_literal(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<std::net::IpAddr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert!(parse_url("https://example.com/").is_ok());
        assert!(parse_url("http://example.com/").is_ok());
    }

    #[test]
    fn rejects_ftp_and_other_schemes() {
        assert_eq!(
            parse_url("ftp://example.com/x"),
            Err(Error::UnsupportedScheme("ftp".to_owned()))
        );
        assert_eq!(
            parse_url("gopher://example.com/"),
            Err(Error::UnsupportedScheme("gopher".to_owned()))
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            parse_url("not a url at all"),
            Err(Error::InvalidUrl(_))
        ));
    }

    #[test]
    fn classifies_absolute_urls() {
        assert!(matches!(
            classify_user_input("https://example.com/a?b=c"),
            UserInput::Url(_)
        ));
        assert!(matches!(
            classify_user_input("http://localhost:3000/"),
            UserInput::Url(_)
        ));
    }

    #[test]
    fn upgrades_bare_hosts_to_https() {
        match classify_user_input("example.com") {
            UserInput::Url(url) => {
                assert_eq!(url.scheme(), "https");
                assert_eq!(url.host_str(), Some("example.com"));
            }
            UserInput::Search(_) => panic!("expected Url"),
        }
    }

    #[test]
    fn plain_words_are_searches() {
        assert!(matches!(
            classify_user_input("how to bake bread"),
            UserInput::Search(_)
        ));
        assert!(matches!(
            classify_user_input("about:blank"),
            UserInput::Search(_)
        ));
    }
}
