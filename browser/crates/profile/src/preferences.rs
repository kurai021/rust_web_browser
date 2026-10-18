use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub general: General,
    pub privacy: Privacy,
    pub downloads: Downloads,
    pub render: Render,
    pub appearance: Appearance,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub homepage: String,
    pub search_engine: String,
    pub restore_session: bool,
}
impl Default for General {
    fn default() -> Self {
        Self {
            homepage: "about:blank".into(),
            search_engine: "https://duckduckgo.com/?q={query}".into(),
            restore_session: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub adblock_enabled: bool,
    pub third_party_cookies: String,
    pub doh_enabled: bool,
    pub javascript_enabled: bool,
    pub site_javascript: BTreeMap<String, bool>,
}
impl Default for Privacy {
    fn default() -> Self {
        Self {
            adblock_enabled: true,
            third_party_cookies: "block".into(),
            doh_enabled: false,
            javascript_enabled: true,
            site_javascript: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Downloads {
    pub path: String,
    pub ask_every_time: bool,
}
impl Default for Downloads {
    fn default() -> Self {
        Self {
            path: "~/Downloads".into(),
            ask_every_time: true,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Render {
    pub software_fallback: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: String,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "system".into(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), String> {
        if crate::clean_url(&self.general.homepage).is_none() {
            return Err("Homepage must be an HTTP(S) or local about URL".into());
        }
        if self.general.search_engine.len() > 4096
            || !self.general.search_engine.contains("{query}")
        {
            return Err("Search URL must include {query}".into());
        }
        let search = url::Url::parse(&self.general.search_engine.replace("{query}", "test"))
            .map_err(|e| e.to_string())?;
        if !matches!(search.scheme(), "http" | "https") {
            return Err("Search engine must use HTTP(S)".into());
        }
        if !matches!(self.appearance.theme.as_str(), "system" | "light" | "dark") {
            return Err("Theme must be system, light or dark".into());
        }
        if self.downloads.path.len() > 4096 || self.downloads.path.contains('\0') {
            return Err("Invalid downloads path".into());
        }
        if !matches!(self.privacy.third_party_cookies.as_str(), "block" | "allow") {
            return Err("Third-party cookies must be block or allow".into());
        }
        if self.privacy.site_javascript.len() > 1000 {
            return Err("Too many site settings".into());
        }
        Ok(())
    }
    pub fn search_url(&self, query: &str) -> Result<url::Url, String> {
        self.validate()?;
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        url::Url::parse(&self.general.search_engine.replace("{query}", &encoded))
            .map_err(|e| e.to_string())
    }
    pub fn javascript(&self, url: &url::Url) -> bool {
        self.privacy
            .site_javascript
            .get(&url.origin().ascii_serialization())
            .copied()
            .unwrap_or(self.privacy.javascript_enabled)
    }
}
