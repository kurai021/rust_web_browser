use crate::*;
#[derive(Debug, Clone)]
pub struct Entry {
    pub url: Url,
    pub state: serde_json::Value,
}
#[derive(Debug, Clone)]
pub struct History {
    pub entries: Vec<Entry>,
    pub position: usize,
}
impl History {
    pub fn new(url: Url) -> Self {
        Self {
            entries: vec![Entry {
                url,
                state: serde_json::Value::Null,
            }],
            position: 0,
        }
    }
    pub fn current(&self) -> &Entry {
        &self.entries[self.position]
    }
    pub fn push(&mut self, url: Url, state: serde_json::Value, replace: bool) -> Result<()> {
        if !same_origin(&self.current().url, &url) {
            return Err(ApiError::new(
                "SecurityError",
                "history URL must be same-origin",
            ));
        }
        if serde_json::to_vec(&state)
            .map_err(|e| ApiError::new("DataCloneError", e.to_string()))?
            .len()
            > 640 * 1024
        {
            return Err(ApiError::new("QuotaExceededError", "history state"));
        }
        let entry = Entry { url, state };
        if replace {
            self.entries[self.position] = entry;
        } else {
            self.entries.truncate(self.position + 1);
            self.entries.push(entry);
            if self.entries.len() > 100 {
                self.entries.remove(0);
            }
            self.position = self.entries.len() - 1;
        }
        Ok(())
    }
    pub fn go(&mut self, delta: i32) -> Option<Entry> {
        let target = self.position as i64 + i64::from(delta);
        if target < 0 || target >= self.entries.len() as i64 {
            return None;
        }
        self.position = target as usize;
        Some(self.current().clone())
    }
}
