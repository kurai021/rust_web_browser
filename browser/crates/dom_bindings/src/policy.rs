//! Basic script-src/default-src, nonce/hash and unsafe-eval policy.
//! SHA hashing is an approved auxiliary crate, never hand-written crypto.
use base64::Engine;
use sha2::{Digest, Sha256, Sha384, Sha512};
use url::Url;
#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub policies: Vec<Vec<(String, Vec<String>)>>,
}
impl Policy {
    pub fn parse(headers: &[String], document: &html::Document) -> Self {
        let mut sources = headers.to_vec();
        for id in document.get_elements_by_tag_name("meta") {
            if document
                .get_attribute(id, "http-equiv")
                .is_some_and(|v| v.eq_ignore_ascii_case("content-security-policy"))
            {
                if let Some(value) = document.get_attribute(id, "content") {
                    sources.push(value.into());
                }
            }
        }
        Self {
            policies: sources
                .into_iter()
                .map(|source| {
                    let mut directives = Vec::new();
                    for part in source.split(';') {
                        let mut words = part.split_ascii_whitespace();
                        if let Some(name) = words.next() {
                            let name = name.to_ascii_lowercase();
                            if !directives.iter().any(|(n, _)| n == &name) {
                                directives.push((name, words.map(str::to_owned).collect()));
                            }
                        }
                    }
                    directives
                })
                .collect(),
        }
    }
    fn script_sources(policy: &[(String, Vec<String>)]) -> Option<&[String]> {
        policy
            .iter()
            .find(|(n, _)| n == "script-src")
            .or_else(|| policy.iter().find(|(n, _)| n == "default-src"))
            .map(|(_, v)| v.as_slice())
    }
    pub fn eval_allowed(&self) -> bool {
        self.policies.iter().all(|p| {
            Self::script_sources(p)
                .is_none_or(|sources| sources.iter().any(|s| s == "'unsafe-eval'"))
        })
    }
    pub fn inline_allowed(&self, source: &str, nonce: Option<&str>) -> bool {
        self.policies.iter().all(|p| {
            Self::script_sources(p).is_none_or(|sources| {
                let hash_or_nonce = sources
                    .iter()
                    .any(|s| s.starts_with("'nonce-") || s.starts_with("'sha"));
                sources.iter().any(|s| {
                    s == "'unsafe-inline'" && !hash_or_nonce
                        || nonce.is_some_and(|n| s == &format!("'nonce-{n}'"))
                        || hash_matches(s, source)
                })
            })
        })
    }
    pub fn external_allowed(&self, document: &Url, resource: &Url, nonce: Option<&str>) -> bool {
        if !matches!(resource.scheme(), "http" | "https")
            || document.scheme() == "https" && resource.scheme() == "http"
        {
            return false;
        }
        self.policies.iter().all(|p| {
            Self::script_sources(p).is_none_or(|sources| {
                sources.iter().any(|s| {
                    if nonce.is_some_and(|n| s == &format!("'nonce-{n}'")) {
                        return true;
                    }
                    if s == "*" {
                        return true;
                    }
                    if s == "'self'" {
                        return document.origin() == resource.origin();
                    }
                    if s == "https:" || s == "http:" {
                        return resource.scheme() == s.trim_end_matches(':');
                    }
                    let source = if s.contains("://") {
                        s.clone()
                    } else {
                        format!("{}://{s}", document.scheme())
                    };
                    Url::parse(&source).ok().is_some_and(|allowed| {
                        allowed.scheme() == resource.scheme()
                            && allowed.port_or_known_default() == resource.port_or_known_default()
                            && allowed.host_str() == resource.host_str()
                            && (allowed.path() == "/"
                                || resource.path().starts_with(allowed.path()))
                    })
                })
            })
        })
    }
}
fn hash_matches(policy: &str, source: &str) -> bool {
    let Some(raw) = policy.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) else {
        return false;
    };
    let Some((kind, want)) = raw.split_once('-') else {
        return false;
    };
    let digest = match kind {
        "sha256" => Sha256::digest(source.as_bytes()).to_vec(),
        "sha384" => Sha384::digest(source.as_bytes()).to_vec(),
        "sha512" => Sha512::digest(source.as_bytes()).to_vec(),
        _ => return false,
    };
    base64::engine::general_purpose::STANDARD
        .decode(want)
        .is_ok_and(|bytes| bytes == digest)
}
