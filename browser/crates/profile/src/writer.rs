use crate::{Data, Preferences, MAX_PROFILE_BYTES};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
};

pub fn default_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|base| base.join("navegador-rust"))
}
pub fn expand_home(path: &str) -> PathBuf {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or(""));
        }
    }
    PathBuf::from(path)
}
fn read_capped(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(io::Error::other("Profile file exceeds 16 MiB"));
    }
    Ok(Some(bytes))
}
/// Atomic replacement; all writes are confined to the selected profile directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(io::Error::other("Profile quota exceeded"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing profile parent"))?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
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
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}
#[derive(Default)]
struct Pending {
    snapshot: Option<(Data, Preferences)>,
    stop: bool,
    error: Option<String>,
}
/// UI-owned in-memory model with a coalescing, one-snapshot background writer.
/// Opening is startup-only; ordinary edits never perform disk I/O on the UI thread.
pub struct Store {
    pub data: Data,
    pub preferences: Preferences,
    pub directory: Option<PathBuf>,
    pending: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}
impl Store {
    pub fn open(directory: Option<PathBuf>) -> io::Result<Self> {
        let mut data = Data::default();
        let mut preferences = Preferences::default();
        if let Some(dir) = &directory {
            if let Some(bytes) = read_capped(&dir.join("browser.json"))? {
                data = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            }
            if let Some(bytes) = read_capped(&dir.join("config.toml"))? {
                preferences =
                    toml::from_str(std::str::from_utf8(&bytes).map_err(io::Error::other)?)
                        .map_err(io::Error::other)?;
            }
        }
        data.normalize();
        preferences.validate().map_err(io::Error::other)?;
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = if let Some(dir) = directory.clone() {
            let shared = pending.clone();
            Some(
                std::thread::Builder::new()
                    .name("profile-writer".into())
                    .spawn(move || loop {
                        let (lock, wake) = &*shared;
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while state.snapshot.is_none() && !state.stop {
                            state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                        let snapshot = state.snapshot.take();
                        let stop = state.stop;
                        drop(state);
                        if let Some((mut data, preferences)) = snapshot {
                            data.normalize();
                            let result = (|| {
                                let bytes = serde_json::to_vec(&data).map_err(io::Error::other)?;
                                atomic_write(&dir.join("browser.json"), &bytes)?;
                                let config = toml::to_string_pretty(&preferences)
                                    .map_err(io::Error::other)?;
                                atomic_write(&dir.join("config.toml"), config.as_bytes())
                            })();
                            if let Err(error) = result {
                                lock.lock().unwrap_or_else(|e| e.into_inner()).error =
                                    Some(error.to_string());
                            }
                        }
                        if stop {
                            break;
                        }
                    })?,
            )
        } else {
            None
        };
        Ok(Self {
            data,
            preferences,
            directory,
            pending,
            worker,
        })
    }
    pub fn changed(&self) {
        if self.worker.is_none() {
            return;
        }
        let (lock, wake) = &*self.pending;
        lock.lock().unwrap_or_else(|e| e.into_inner()).snapshot =
            Some((self.data.clone(), self.preferences.clone()));
        wake.notify_one();
    }
    pub fn take_error(&self) -> Option<String> {
        self.pending
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .take()
    }
    pub fn shutdown(&mut self) {
        if self.worker.is_none() {
            return;
        }
        self.changed();
        self.pending
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop = true;
        self.pending.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        self.shutdown();
    }
}
