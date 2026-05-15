use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::Error;
use crate::telemetry::prelude::*;

const FILE_NAME: &str = "autosync_settings.json";
const LEGACY_FILE_NAME: &str = "autopull_settings.json";

/// User-configurable knobs for the background autosync watcher.
///
/// Persisted as `autosync_settings.json` in `app_local_data_dir`. `enabled`
/// defaults to `false` — the loop is opt-in until the autosync UX is
/// finalised. The `closed_secs` field is kept on disk from day one so we
/// don't break the file format when the tray-icon milestone lands.
///
/// `enabled` governs both the pull and the push directions of the loop.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AutosyncSettings {
    pub enabled: bool,
    pub focused_secs: u64,
    pub unfocused_secs: u64,
    pub closed_secs: u64,
}

impl Default for AutosyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            focused_secs: 30,
            unfocused_secs: 120,
            closed_secs: 600,
        }
    }
}

impl AutosyncSettings {
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

pub type SharedAutosyncSettings = Arc<RwLock<AutosyncSettings>>;

pub async fn init(data_dir: &Path) -> Result<SharedAutosyncSettings, Error> {
    // Best-effort migration from the legacy file name. The JSON shape is
    // unchanged, so a single `rename` is byte-identical to a round-trip
    // through serde — and avoids losing user state on a partial
    // deserialise. `NotFound` is the normal case (fresh install or
    // already migrated); anything else is logged and the load proceeds.
    let legacy = data_dir.join(LEGACY_FILE_NAME);
    let target = data_dir.join(FILE_NAME);
    if !target.exists() {
        match tokio::fs::rename(&legacy, &target).await {
            Ok(()) => {
                info!("autosync: migrated {LEGACY_FILE_NAME} → {FILE_NAME}");
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    "autosync: failed to migrate {LEGACY_FILE_NAME} → {FILE_NAME}: {err}; \
                     continuing with whatever exists",
                );
            }
        }
    }
    let settings = AutosyncSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn roundtrip_under_new_name() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = AutosyncSettings {
            enabled: true,
            focused_secs: 5,
            unfocused_secs: 60,
            closed_secs: 300,
        };
        settings.save(dir.path()).await?;
        assert!(dir.path().join(FILE_NAME).exists());
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(loaded, settings);
        Ok(())
    }

    #[tokio::test]
    async fn missing_file_returns_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(loaded, AutosyncSettings::default());
        assert!(!loaded.enabled);
        Ok(())
    }

    #[tokio::test]
    async fn legacy_file_is_migrated() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let legacy_path = dir.path().join(LEGACY_FILE_NAME);
        let new_path = dir.path().join(FILE_NAME);

        // Write an arbitrary JSON blob — migration is a single `rename`,
        // so the test does not need a typed `AutosyncSettings`.
        let payload = br#"{"enabled":true,"focused_secs":7,"unfocused_secs":70,"closed_secs":700}"#;
        tokio::fs::write(&legacy_path, payload).await?;

        let _shared = init(dir.path()).await?;

        assert!(!legacy_path.exists(), "legacy file should be gone");
        let migrated = tokio::fs::read(&new_path).await?;
        assert_eq!(migrated, payload, "migration must be byte-identical");
        Ok(())
    }
}
