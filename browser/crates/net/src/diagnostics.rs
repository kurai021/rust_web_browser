//! Bounded, in-memory connection details for trusted browser chrome only.
use crate::Client;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub id: u64,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub received: u64,
    pub state: String,
}
pub(crate) struct Trace {
    id: u64,
    records: Arc<Mutex<VecDeque<RequestRecord>>>,
    done: bool,
}
impl Trace {
    fn edit(&self, f: impl FnOnce(&mut RequestRecord)) {
        if let Some(row) = self
            .records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
            .find(|r| r.id == self.id)
        {
            f(row);
        }
    }
    pub(crate) fn headers(&self, status: u16) {
        self.edit(|r| {
            r.status = Some(status);
            r.state = "Receiving".into();
        });
    }
    pub(crate) fn bytes(&self, n: usize) {
        self.edit(|r| r.received = r.received.saturating_add(n as u64));
    }
    pub(crate) fn finish(&mut self) {
        self.done = true;
        self.edit(|r| r.state = "Complete".into());
    }
    pub(crate) fn fail(&mut self, error: &str) {
        self.done = true;
        self.edit(|r| r.state = error.chars().take(512).collect());
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        if !self.done {
            self.edit(|r| r.state = "Cancelled or interrupted".into());
        }
    }
}
impl Client {
    pub(crate) fn begin_request(&self, method: &str, url: &url::Url) -> Trace {
        let id = self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let mut public = format!("{}://", url.scheme());
        public.extend(url[url::Position::BeforeHost..].chars().take(8192));
        let mut records = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        if records.len() >= 256 {
            records.pop_front();
        }
        records.push_back(RequestRecord {
            id,
            method: method.chars().take(64).collect(),
            url: public,
            status: None,
            received: 0,
            state: "Connecting".into(),
        });
        Trace {
            id,
            records: self.requests.clone(),
            done: false,
        }
    }
    pub fn request_log(&self) -> Vec<RequestRecord> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
    pub fn certificate_details(&self, url: &url::Url) -> Option<String> {
        self.certificates
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&url.origin().ascii_serialization())
            .cloned()
    }
    pub(crate) fn note_certificate(&self, url: &url::Url, response: &reqwest::Response) {
        if url.scheme() != "https" {
            return;
        }
        let Some(der) = response
            .extensions()
            .get::<reqwest::tls::TlsInfo>()
            .and_then(|info| info.peer_certificate())
        else {
            return;
        };
        if der.len() > 64 * 1024 {
            return;
        }
        use sha2::Digest;
        let fingerprint = sha2::Sha256::digest(der)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        let mut details = format!(
            "Hostname: {}\nCertificate verified by rustls\nSHA-256: {fingerprint}",
            url.host_str().unwrap_or("")
        );
        if let Ok((_, cert)) = x509_parser::parse_x509_certificate(der) {
            details = format!(
                "Subject: {}\nIssuer: {}\nValid from: {}\nValid until: {}\n{details}",
                cert.subject(),
                cert.issuer(),
                cert.validity().not_before,
                cert.validity().not_after
            );
        }
        let mut cache = self.certificates.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 128 {
            cache.pop_first();
        }
        cache.insert(url.origin().ascii_serialization(), details);
    }
}
