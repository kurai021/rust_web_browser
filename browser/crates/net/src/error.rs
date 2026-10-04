//! Typed network errors (plan/04 §4.3.3, §4.5).
//!
//! Every failure the UI can show maps to [`Error`]; TLS failures map to
//! [`TlsError`] by walking the underlying `reqwest` source chain looking
//! for `rustls` errors. Nothing here panics on hostile input.

use std::error::Error as StdError;

use thiserror::Error as ThisError;

/// User-facing network failure.
#[derive(Debug, ThisError, Clone, PartialEq, Eq)]
pub enum Error {
    /// URL scheme is not `http`/`https` (plan/01.4: FTP/Gopher/etc. rejected).
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    /// The text is not a parseable URL at all.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// DNS resolution failed.
    #[error("DNS lookup failed for {0}")]
    Dns(String),
    /// TCP connect failed (host unreachable / refused).
    #[error("connection failed: {0}")]
    Connect(String),
    /// TLS handshake / verification failed.
    #[error("TLS error: {0}")]
    Tls(#[from] TlsError),
    /// Connect, TTFB or total deadline exceeded.
    #[error("timed out: {0}")]
    Timeout(String),
    /// More than the allowed redirect hops.
    #[error("too many redirects")]
    TooManyRedirects,
    /// HTTP status the navigator refuses to render (4xx/5xx surfaced, not hidden).
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    /// Document/resource exceeded the byte cap (zip-bomb guard, plan/04 §4.3.2).
    #[error("response too large (capped at {cap} bytes)")]
    BodyTooLarge { cap: usize },
    /// Fetch was cancelled (tab closed, user pressed stop).
    #[error("cancelled")]
    Cancelled,
    /// Anything else from the transport, kept as a string (never leaks internals to pages).
    #[error("transport error: {0}")]
    Transport(String),
}

/// TLS failure details shown by the certificate error page.
#[derive(Debug, ThisError, Clone, PartialEq, Eq)]
pub enum TlsError {
    #[error("certificate expired (CERT_EXPIRED)")]
    Expired,
    #[error("certificate not yet valid (CERT_NOT_YET_VALID)")]
    NotYetValid,
    #[error("unknown issuer — possibly self-signed (SELF_SIGNED?)")]
    UnknownIssuer,
    #[error("certificate not valid for this host (WRONG_HOST)")]
    WrongHost,
    #[error("certificate revoked (REVOKED)")]
    Revoked,
    #[error("insecure protocol version or cipher (TLS_VERSION)")]
    InsecureVersion,
    #[error("TLS failure: {0}")]
    Other(String),
}

/// Map a `reqwest` failure to [`Error`], digging for `rustls` causes.
#[must_use]
pub fn map_reqwest(err: &reqwest::Error) -> Error {
    if err.is_timeout() {
        return Error::Timeout(err.to_string());
    }
    if err.is_redirect() {
        return Error::TooManyRedirects;
    }
    if err.is_body() || err.is_decode() {
        return Error::Transport(format!("body error: {err}"));
    }
    if err.is_connect() {
        if let Some(tls) = find_tls_error(err) {
            return Error::Tls(tls);
        }
        let text = err.to_string();
        if is_dns_hint(&text) {
            return Error::Dns(text);
        }
        return Error::Connect(text);
    }
    if let Some(tls) = find_tls_error(err) {
        return Error::Tls(tls);
    }
    Error::Transport(err.to_string())
}

/// Walk up to 8 `source()` levels collecting the chained `Display` text,
/// then classify it.
///
/// `hyper`/`reqwest` surface certificate failures as text (e.g. "invalid
/// peer certificate: Expired") buried inside `io::Error` wrappers that do
/// not forward typed providers, so keyword matching over the chain is the
/// version-proof mapping. It only fires when the chain already mentions
/// TLS, so plain connect errors never misclassify. End-to-end coverage:
/// `tests/fetch_online.rs` against badssl.com.
fn find_tls_error(err: &dyn StdError) -> Option<TlsError> {
    let mut chain_text = String::new();
    let mut current: Option<&dyn StdError> = Some(err);
    for _ in 0..8 {
        let Some(node) = current else { break };
        chain_text.push_str(&node.to_string());
        chain_text.push('\n');
        current = node.source();
    }
    classify_tls_text(&chain_text)
}

/// Keyword fallback for TLS failures visible only as text.
fn classify_tls_text(text: &str) -> Option<TlsError> {
    let lower = text.to_lowercase();
    const MARKERS: [&str; 7] = [
        "certificate",
        "tls",
        "ssl",
        "rustls",
        "webpki",
        "handshake",
        "pki",
    ];
    if !MARKERS.iter().any(|m| lower.contains(m)) {
        return None;
    }
    if lower.contains("expir") {
        Some(TlsError::Expired)
    } else if lower.contains("not valid for") || lower.contains("notvalidforname") {
        Some(TlsError::WrongHost)
    } else if lower.contains("unknown issuer")
        || lower.contains("unknownissuer")
        || lower.contains("self-sign")
        || lower.contains("selfsign")
    {
        Some(TlsError::UnknownIssuer)
    } else if lower.contains("not yet valid") || lower.contains("notvalidyet") {
        Some(TlsError::NotYetValid)
    } else if lower.contains("revok") {
        Some(TlsError::Revoked)
    } else if lower.contains("handshake failure")
        || lower.contains("protocol version")
        || lower.contains("no cipher")
        || lower.contains("cipher suite")
        || lower.contains("ciphersuite")
    {
        Some(TlsError::InsecureVersion)
    } else {
        let first: String = text
            .lines()
            .next()
            .unwrap_or("TLS failure")
            .chars()
            .take(200)
            .collect();
        Some(TlsError::Other(first))
    }
}

/// Heuristic: `reqwest` connect errors embed the resolver message as text.
fn is_dns_hint(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("dns")
        || lower.contains("name resolution")
        || lower.contains("failed to lookup")
        || lower.contains("no addresses")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::CertificateError;
    use std::fmt;
    use std::io::ErrorKind;

    /// Mimics a real transport chain: outer handshake message + rustls cause.
    #[derive(Debug)]
    struct TlsWrap {
        source: std::io::Error,
    }

    impl fmt::Display for TlsWrap {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "error trying to connect: tls handshake failed")
        }
    }

    impl StdError for TlsWrap {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.source)
        }
    }

    fn chain_with(rustls_err: rustls::Error) -> TlsWrap {
        TlsWrap {
            source: std::io::Error::other(rustls_err),
        }
    }

    #[test]
    fn finds_expired_in_chain() {
        let chained = chain_with(rustls::Error::InvalidCertificate(CertificateError::Expired));
        assert_eq!(find_tls_error(&chained), Some(TlsError::Expired));
    }

    #[test]
    fn finds_wrong_host_in_chain() {
        let chained = chain_with(rustls::Error::InvalidCertificate(
            CertificateError::NotValidForName,
        ));
        assert_eq!(find_tls_error(&chained), Some(TlsError::WrongHost));
    }

    #[test]
    fn finds_unknown_issuer_in_chain() {
        let chained = chain_with(rustls::Error::InvalidCertificate(
            CertificateError::UnknownIssuer,
        ));
        assert_eq!(find_tls_error(&chained), Some(TlsError::UnknownIssuer));
    }

    #[test]
    fn no_tls_error_without_tls_markers() {
        let plain = std::io::Error::new(ErrorKind::ConnectionRefused, "refused");
        assert_eq!(find_tls_error(&plain), None);
        // Bare rustls text still carries its own TLS marker ("certificate"),
        // so it classifies — the gate only stops markerless errors.
        let bare =
            std::io::Error::other(rustls::Error::InvalidCertificate(CertificateError::Expired));
        assert_eq!(find_tls_error(&bare), Some(TlsError::Expired));
    }

    #[test]
    fn dns_hint_detection() {
        assert!(is_dns_hint("dns error: failed to lookup address"));
        assert!(!is_dns_hint("connection refused"));
    }
}
