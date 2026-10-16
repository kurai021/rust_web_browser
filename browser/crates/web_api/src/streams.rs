//! Bounded linear-time event-stream framing across arbitrary UTF-8/CRLF chunks.
use crate::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerEvent {
    pub event: String,
    pub data: String,
    pub id: String,
}
pub struct SseParser {
    pending: Vec<u8>,
    data: Vec<String>,
    data_bytes: usize,
    event: String,
    id: String,
    skip_lf: bool,
    first: bool,
    retry: u64,
}
impl Default for SseParser {
    fn default() -> Self {
        Self::with_id(String::new())
    }
}
impl SseParser {
    pub fn with_id(id: String) -> Self {
        Self {
            pending: Vec::new(),
            data: Vec::new(),
            data_bytes: 0,
            event: String::new(),
            id,
            skip_lf: false,
            first: true,
            retry: 3000,
        }
    }
    pub fn last_id(&self) -> &str {
        &self.id
    }
    pub fn retry_ms(&self) -> u64 {
        self.retry
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<ServerEvent>> {
        let mut events = Vec::new();
        for &byte in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\n' || byte == b'\r' {
                self.skip_lf = byte == b'\r';
                let raw = std::mem::take(&mut self.pending);
                let line = String::from_utf8_lossy(&raw);
                let line = if self.first {
                    self.first = false;
                    line.trim_start_matches('\u{feff}')
                } else {
                    &line
                };
                if line.is_empty() {
                    if !self.data.is_empty() {
                        if events.len() >= 1024 {
                            return Err(ApiError::new("QuotaExceededError", "SSE event batch"));
                        }
                        events.push(ServerEvent {
                            event: if self.event.is_empty() {
                                "message".into()
                            } else {
                                self.event.clone()
                            },
                            data: self.data.join("\n"),
                            id: self.id.clone(),
                        });
                    }
                    self.data.clear();
                    self.data_bytes = 0;
                    self.event.clear();
                } else if !line.starts_with(':') {
                    let (field, value) = line.split_once(':').unwrap_or((line, ""));
                    let value = value.strip_prefix(' ').unwrap_or(value);
                    match field {
                        "data" => {
                            self.data_bytes = self.data_bytes.saturating_add(value.len() + 1);
                            if self.data_bytes > 1024 * 1024 {
                                return Err(ApiError::new("QuotaExceededError", "SSE event quota"));
                            }
                            self.data.push(value.into());
                        }
                        "event" => self.event = value.into(),
                        "id" if !value.contains('\0') => self.id = value.into(),
                        "retry"
                            if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) =>
                        {
                            if let Ok(n) = value.parse::<u64>() {
                                self.retry = n.clamp(1000, 60_000);
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                if self.pending.len() >= 1024 * 1024 {
                    return Err(ApiError::new("QuotaExceededError", "SSE line quota"));
                }
                self.pending.push(byte);
            }
        }
        Ok(events)
    }
}
