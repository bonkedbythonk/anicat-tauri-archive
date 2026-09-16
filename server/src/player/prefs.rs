//! The player settings the page and mpv's keys both change: `prefs.json` in
//! the data dir. Defaults are the Mac's (`anicat_gpu_upscaling`,
//! `anicat_autoskip`, `anicat_autoplay_next`, all on for macOS).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub upscaling: bool,
    pub autoskip: bool,
    pub auto_next: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { upscaling: true, autoskip: true, auto_next: true }
    }
}

/// A partial update, as `POST /api/prefs` sends it.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct PrefsPatch {
    pub upscaling: Option<bool>,
    pub autoskip: Option<bool>,
    pub auto_next: Option<bool>,
}

impl PrefsPatch {
    pub fn apply(self, p: &mut Prefs) {
        if let Some(v) = self.upscaling {
            p.upscaling = v;
        }
        if let Some(v) = self.autoskip {
            p.autoskip = v;
        }
        if let Some(v) = self.auto_next {
            p.auto_next = v;
        }
    }
}

pub struct PrefsStore {
    path: PathBuf,
    /// Held across the change *and* the file write. Two toggles racing (a key
    /// in mpv and a click on the page) could otherwise write their snapshots
    /// in the opposite order and leave the file saying the older value.
    current: Mutex<Prefs>,
}

impl PrefsStore {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("prefs.json");
        let prefs = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| match serde_json::from_str::<Prefs>(&t) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::warn!("[prefs] {} unreadable, using defaults: {e}", path.display());
                    None
                }
            })
            .unwrap_or_default();
        Self { path, current: Mutex::new(prefs) }
    }

    pub fn get(&self) -> Prefs {
        self.current.lock().map(|p| *p).unwrap_or_default()
    }

    /// Changes, persists and returns the new value. Blocking: a small file
    /// write, called through `spawn_blocking` from async code.
    pub fn update(&self, f: impl FnOnce(&mut Prefs)) -> Prefs {
        let Ok(mut guard) = self.current.lock() else { return Prefs::default() };
        f(&mut guard);
        let snapshot = *guard;
        if let Err(e) = write(&self.path, &snapshot) {
            log::warn!("[prefs] could not save {}: {e}", self.path.display());
        }
        snapshot
    }
}

/// Write-then-rename, as `config::save_token` does: a crash mid-write would
/// otherwise leave a truncated file, and every setting would silently reset.
fn write(path: &Path, prefs: &Prefs) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(prefs).unwrap_or_default())?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anicat-prefs-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn defaults_when_missing_and_round_trips() {
        let dir = temp_dir("roundtrip");
        let store = PrefsStore::load(&dir);
        assert_eq!(store.get(), Prefs::default());
        let after = store.update(|p| p.upscaling = false);
        assert!(!after.upscaling && after.autoskip && after.auto_next);
        assert!(!dir.join("prefs.json.tmp").exists());
        assert_eq!(PrefsStore::load(&dir).get(), after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_keys_take_defaults_and_garbage_is_ignored() {
        let dir = temp_dir("partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("prefs.json"), br#"{"auto_next": false}"#).unwrap();
        assert_eq!(PrefsStore::load(&dir).get(), Prefs { upscaling: true, autoskip: true, auto_next: false });
        std::fs::write(dir.join("prefs.json"), b"{trunc").unwrap();
        assert_eq!(PrefsStore::load(&dir).get(), Prefs::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn patch_changes_only_named_fields() {
        let mut p = Prefs::default();
        PrefsPatch { autoskip: Some(false), ..Default::default() }.apply(&mut p);
        assert_eq!(p, Prefs { upscaling: true, autoskip: false, auto_next: true });
    }
}
