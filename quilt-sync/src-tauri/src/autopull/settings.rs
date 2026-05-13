use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::Error;

const FILE_NAME: &str = "autopull_settings.json";

/// User-configurable knobs for the background autopull watcher.
///
/// Persisted as `autopull_settings.json` in `app_local_data_dir`. `enabled`
/// defaults to `false` — the loop is opt-in until autopush and the conflict
/// UI ship. The `closed_secs` field is kept on disk from day one so we don't
/// break the file format when the tray-icon milestone lands.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AutopullSettings {
    pub enabled: bool,
    pub focused_secs: u64,
    pub unfocused_secs: u64,
    pub closed_secs: u64,
}

impl Default for AutopullSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            focused_secs: 30,
            unfocused_secs: 120,
            closed_secs: 600,
        }
    }
}

impl AutopullSettings {
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

pub type SharedAutopullSettings = Arc<RwLock<AutopullSettings>>;

pub async fn init(data_dir: &Path) -> Result<SharedAutopullSettings, Error> {
    let settings = AutopullSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn roundtrip() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = AutopullSettings {
            enabled: true,
            focused_secs: 5,
            unfocused_secs: 60,
            closed_secs: 300,
        };
        settings.save(dir.path()).await?;
        let loaded = AutopullSettings::load(dir.path()).await?;
        assert_eq!(loaded, settings);
        Ok(())
    }

    #[tokio::test]
    async fn missing_file_returns_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = AutopullSettings::load(dir.path()).await?;
        assert_eq!(loaded, AutopullSettings::default());
        assert!(!loaded.enabled);
        Ok(())
    }
}
