use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::Error;

const FILE_NAME: &str = "fswatcher_settings.json";

const DEFAULT_DEBOUNCE_MS: u64 = 500;

fn default_debounce_ms() -> u64 {
    DEFAULT_DEBOUNCE_MS
}

fn default_enabled() -> bool {
    true
}

/// User-configurable knobs for the filesystem watcher.
///
/// Persisted as `fswatcher_settings.json` in `app_local_data_dir`.
/// `enabled` defaults to `true` — the watcher is read-only and updates only
/// local state, so the opt-in bar is lower than autopull. `debounce_ms` is
/// stored on disk but not surfaced in the UI for milestone 4; promoting it
/// later is zero-migration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FsWatcherSettings {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
}

impl Default for FsWatcherSettings {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            debounce_ms: default_debounce_ms(),
        }
    }
}

impl FsWatcherSettings {
    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    pub async fn load(data_dir: &Path) -> Result<Self, Error> {
        let path = Self::file_path(data_dir);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(Error::from(err)),
        }
    }

    pub async fn save(&self, data_dir: &Path) -> Result<(), Error> {
        tokio::fs::create_dir_all(data_dir).await?;
        let path = Self::file_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
}

pub type SharedFsWatcherSettings = Arc<RwLock<FsWatcherSettings>>;

pub async fn init(data_dir: &Path) -> Result<SharedFsWatcherSettings, Error> {
    let settings = FsWatcherSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn defaults() {
        let s = FsWatcherSettings::default();
        assert!(s.enabled);
        assert_eq!(s.debounce_ms, 500);
    }

    #[tokio::test]
    async fn roundtrip() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = FsWatcherSettings {
            enabled: false,
            debounce_ms: 1000,
        };
        settings.save(dir.path()).await?;
        let loaded = FsWatcherSettings::load(dir.path()).await?;
        assert_eq!(loaded, settings);
        Ok(())
    }

    #[tokio::test]
    async fn missing_file_returns_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = FsWatcherSettings::load(dir.path()).await?;
        assert_eq!(loaded, FsWatcherSettings::default());
        Ok(())
    }

    #[tokio::test]
    async fn forward_compat_missing_fields_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(dir.path().join(FILE_NAME), b"{}").await?;
        let loaded = FsWatcherSettings::load(dir.path()).await?;
        assert_eq!(loaded, FsWatcherSettings::default());
        Ok(())
    }
}
