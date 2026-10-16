//! Profile persistence is trusted Rust I/O; page-controlled strings never form paths.
use crate::*;
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
pub const STORAGE_QUOTA: usize = 5 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Database {
    #[serde(default)]
    pub generation: u64,
    pub version: u64,
    pub stores: BTreeMap<String, ObjectStore>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectStore {
    pub key_path: Option<String>,
    pub auto_increment: bool,
    pub next_key: u64,
    pub records: BTreeMap<String, serde_json::Value>,
    pub indexes: BTreeMap<String, Index>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub key_path: String,
    pub unique: bool,
    pub values: BTreeMap<String, Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OriginData {
    pub local: BTreeMap<String, String>,
    pub databases: BTreeMap<String, Database>,
    pub permissions: BTreeMap<String, String>,
}
#[derive(Default, Serialize, Deserialize)]
struct Data {
    origins: BTreeMap<String, OriginData>,
    #[serde(skip)]
    sessions: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
}
#[derive(Default)]
pub struct Profile {
    path: Option<PathBuf>,
    data: Mutex<Data>,
}
fn size(values: &BTreeMap<String, String>) -> usize {
    values
        .iter()
        .map(|(k, v)| (k.encode_utf16().count() + v.encode_utf16().count()) * 2)
        .sum()
}
impl Profile {
    pub fn is_persistent(&self) -> bool {
        self.path.is_some()
    }
    pub fn open(directory: Option<PathBuf>) -> Result<Self> {
        let path = directory.map(|d| d.join("web-data.json"));
        let data = if let Some(path) = &path {
            if path.exists() {
                if path.metadata().map_err(io)?.len() > 64 * 1024 * 1024 {
                    return Err(ApiError::new("QuotaExceededError", "profile size"));
                }
                serde_json::from_slice(&std::fs::read(path).map_err(io)?)
                    .map_err(|e| ApiError::new("DataError", e.to_string()))?
            } else {
                Data::default()
            }
        } else {
            Data::default()
        };
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }
    fn persist(&self, data: &Data) -> Result<()> {
        let bytes =
            serde_json::to_vec(data).map_err(|e| ApiError::new("DataError", e.to_string()))?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(ApiError::new("QuotaExceededError", "profile quota"));
        }
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            let tmp = path.with_extension("tmp");
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp).map_err(io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    .map_err(io)?;
            }
            std::io::Write::write_all(&mut file, &bytes).map_err(io)?;
            std::fs::rename(tmp, path).map_err(io)?;
        }
        Ok(())
    }
    pub fn storage(&self, url: &Url, session: Option<&str>) -> Result<BTreeMap<String, String>> {
        let origin = origin(url)?;
        let d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        Ok(if let Some(s) = session {
            d.sessions
                .get(s)
                .and_then(|m| m.get(&origin))
                .cloned()
                .unwrap_or_default()
        } else {
            d.origins
                .get(&origin)
                .map(|o| o.local.clone())
                .unwrap_or_default()
        })
    }
    pub fn storage_set(
        &self,
        url: &Url,
        session: Option<&str>,
        key: Option<&str>,
        value: Option<&str>,
    ) -> Result<()> {
        let origin = origin(url)?;
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let old = if let Some(s) = session {
            d.sessions
                .entry(s.into())
                .or_default()
                .entry(origin.clone())
                .or_default()
        } else {
            &mut d.origins.entry(origin).or_default().local
        };
        let mut updated = old.clone();
        match (key, value) {
            (Some(k), Some(v)) => {
                updated.insert(k.into(), v.into());
            }
            (Some(k), None) => {
                updated.remove(k);
            }
            (None, _) => updated.clear(),
        };
        if size(&updated) > STORAGE_QUOTA {
            return Err(ApiError::new(
                "QuotaExceededError",
                "5 MiB origin storage quota",
            ));
        }
        let backup = std::mem::replace(old, updated);
        if let Err(error) = self.persist(&d) {
            let old = if let Some(s) = session {
                d.sessions
                    .entry(s.into())
                    .or_default()
                    .entry(crate::origin(url)?)
                    .or_default()
            } else {
                &mut d.origins.entry(crate::origin(url)?).or_default().local
            };
            *old = backup;
            return Err(error);
        }
        Ok(())
    }
    pub fn close_session(&self, id: &str) -> Result<()> {
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        d.sessions.remove(id);
        self.persist(&d)
    }
    pub fn estimate(&self, url: &Url) -> Result<(usize, usize)> {
        let key = origin(url)?;
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let usage = if let Some(data) = data.origins.get(&key) {
            size(&data.local)
                + serde_json::to_vec(&data.databases)
                    .map_err(|e| ApiError::new("DataError", e.to_string()))?
                    .len()
        } else {
            0
        };
        Ok((usage, STORAGE_QUOTA + 32 * 1024 * 1024))
    }
    pub fn permission(&self, url: &Url, name: &str) -> Option<String> {
        let key = origin(url).ok()?;
        self.data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .origins
            .get(&key)?
            .permissions
            .get(name)
            .cloned()
    }
    pub fn set_permission(&self, url: &Url, name: &str, value: &str) -> Result<()> {
        let key = origin(url)?;
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let backup = d.origins.get(&key).cloned();
        d.origins
            .entry(key.clone())
            .or_default()
            .permissions
            .insert(name.into(), value.into());
        if let Err(e) = self.persist(&d) {
            if let Some(old) = backup {
                d.origins.insert(key, old);
            } else {
                d.origins.remove(&key);
            }
            return Err(e);
        }
        Ok(())
    }
    pub fn revoke_permissions(&self) -> Result<()> {
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        for origin in d.origins.values_mut() {
            origin.permissions.clear();
        }
        self.persist(&d)
    }
    pub fn revoke_origin_permissions(&self, url: &Url) -> Result<()> {
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(data) = d.origins.get_mut(&origin(url)?) {
            data.permissions.clear();
        }
        self.persist(&d)
    }
    pub fn database(&self, url: &Url, name: &str) -> Result<Database> {
        let key = origin(url)?;
        let d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        Ok(d.origins
            .get(&key)
            .and_then(|o| o.databases.get(name))
            .cloned()
            .unwrap_or_default())
    }
    pub fn save_database(&self, url: &Url, name: &str, mut value: Database) -> Result<()> {
        let key = origin(url)?;
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let data = d.origins.entry(key).or_default();
        if data.databases.get(name).map_or(0, |db| db.generation) != value.generation {
            return Err(ApiError::new("AbortError", "concurrent database update"));
        }
        value.generation = value
            .generation
            .checked_add(1)
            .ok_or_else(|| ApiError::new("QuotaExceededError", "database generations"))?;
        let mut updated = data.databases.clone();
        updated.insert(name.into(), value);
        if serde_json::to_vec(&updated)
            .map_err(|e| ApiError::new("DataError", e.to_string()))?
            .len()
            > 32 * 1024 * 1024
        {
            return Err(ApiError::new("QuotaExceededError", "IndexedDB quota"));
        }
        let old = std::mem::replace(&mut data.databases, updated);
        if let Err(e) = self.persist(&d) {
            d.origins.entry(origin(url)?).or_default().databases = old;
            return Err(e);
        }
        Ok(())
    }
    pub fn delete_database(&self, url: &Url, name: &str) -> Result<()> {
        let key = origin(url)?;
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        d.origins.entry(key).or_default().databases.remove(name);
        self.persist(&d)
    }
}
fn io(e: std::io::Error) -> ApiError {
    ApiError::new("UnknownError", e.to_string())
}
