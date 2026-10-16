use crate::*;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Headers(pub Vec<(String, String)>);
impl Headers {
    pub fn validate_name(name: &str) -> Result<String> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return Err(ApiError::new("TypeError", "invalid header name"));
        }
        Ok(name.to_ascii_lowercase())
    }
    pub fn append(&mut self, name: &str, value: &str) -> Result<()> {
        let name = Self::validate_name(name)?;
        let value = value.trim_matches([' ', '\t']);
        if value.len() > 16384 || value.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n') {
            return Err(ApiError::new("TypeError", "invalid header value"));
        }
        if self.0.len() >= 256 {
            return Err(ApiError::new("QuotaExceededError", "header count"));
        }
        self.0.push((name, value.into()));
        Ok(())
    }
    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        let name = Self::validate_name(name)?;
        let mut test = Self::default();
        test.append(&name, value)?;
        self.delete(&name);
        self.append(&name, value)
    }
    pub fn delete(&mut self, name: &str) {
        self.0.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }
    pub fn get(&self, name: &str) -> Option<String> {
        let v = self
            .0
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>();
        (!v.is_empty()).then(|| v.join(", "))
    }
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut keys = self.0.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .filter_map(|k| self.get(&k).map(|v| (k, v)))
            .collect()
    }
    pub fn request_forbidden(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        n.starts_with("sec-")
            || n.starts_with("proxy-")
            || matches!(
                n.as_str(),
                "accept-charset"
                    | "accept-encoding"
                    | "access-control-request-headers"
                    | "access-control-request-method"
                    | "connection"
                    | "content-length"
                    | "cookie"
                    | "cookie2"
                    | "date"
                    | "dnt"
                    | "expect"
                    | "host"
                    | "keep-alive"
                    | "origin"
                    | "referer"
                    | "set-cookie"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "upgrade"
                    | "via"
            )
    }
    pub fn safelisted(name: &str, value: &str) -> bool {
        if value.len() > 128 {
            return false;
        }
        match name.to_ascii_lowercase().as_str() {
            "accept" => !value
                .bytes()
                .any(|b| b < 32 && b != 9 || b"\"():<>?@[\\]{}".contains(&b)),
            "accept-language" | "content-language" => value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" *,-.;=".contains(&b)),
            "content-type" => matches!(
                value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
            ),
            _ => false,
        }
    }
}
