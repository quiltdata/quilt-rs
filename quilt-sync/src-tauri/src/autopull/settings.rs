use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::Error;

const FILE_NAME: &str = "autosync_settings.json";

/// User-configurable knobs for the background autosync watcher.
///
/// Persisted as `autosync_settings.json` in `app_local_data_dir`. Both
/// directions default to `false` — the loop is opt-in.
///
/// `pull` and `push` are independent because most users want background
/// pulls (cheap, idempotent) without unattended pushes (commits a
/// snapshot of whatever is on disk to the remote). Splitting them lets
/// pull turn on by default in a future release without implicitly
/// opting users into autopush.
///
/// On disk the JSON is flat — `#[serde(flatten)]` projects the nested
/// Rust struct onto the same keys 0.18.0 wrote, so no migration is
/// required.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AutosyncSettings {
    #[serde(flatten)]
    pub pull: PullSettings,
    #[serde(flatten)]
    pub push: PushSettings,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PullSettings {
    #[serde(default, rename = "pull_enabled")]
    pub enabled: bool,
    pub focused_secs: u64,
    pub unfocused_secs: u64,
    pub closed_secs: u64,
}

impl Default for PullSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            focused_secs: 30,
            unfocused_secs: 120,
            closed_secs: 600,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PushSettings {
    #[serde(default, rename = "push_enabled")]
    pub enabled: bool,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl Default for PushSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_secs: default_idle_timeout_secs(),
        }
    }
}

const fn default_idle_timeout_secs() -> u64 {
    30
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
    let settings = AutosyncSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn nondefault() -> AutosyncSettings {
        AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 5,
                unfocused_secs: 60,
                closed_secs: 300,
            },
            push: PushSettings {
                enabled: true,
                idle_timeout_secs: 45,
            },
        }
    }

    #[tokio::test]
    async fn roundtrip_under_new_shape() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = nondefault();
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
        assert!(!loaded.pull.enabled);
        assert!(!loaded.push.enabled);
        Ok(())
    }

    #[tokio::test]
    async fn flat_json_round_trips() -> Result<(), Error> {
        // The disk shape is flat — `#[serde(flatten)]` projects the nested
        // Rust struct onto the same JSON keys the 0.18.0 release wrote.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "pull_enabled": true,
            "push_enabled": false,
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600,
            "idle_timeout_secs": 45
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert!(loaded.pull.enabled);
        assert!(!loaded.push.enabled);
        assert_eq!(loaded.pull.focused_secs, 30);
        assert_eq!(loaded.pull.unfocused_secs, 120);
        assert_eq!(loaded.pull.closed_secs, 600);
        assert_eq!(loaded.push.idle_timeout_secs, 45);
        Ok(())
    }

    #[tokio::test]
    async fn missing_idle_timeout_defaults_to_30() -> Result<(), Error> {
        // 0.18.0 JSON shape: no `idle_timeout_secs` key.
        // `#[serde(default = "default_idle_timeout_secs")]` must fill in 30.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "pull_enabled": true,
            "push_enabled": true,
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert_eq!(
            loaded.push.idle_timeout_secs, 30,
            "0.18.0 files (no idle_timeout_secs) must load with the 30s default"
        );
        // Other fields preserved verbatim.
        assert!(loaded.pull.enabled);
        assert!(loaded.push.enabled);
        assert_eq!(loaded.pull.focused_secs, 30);
        assert_eq!(loaded.pull.unfocused_secs, 120);
        assert_eq!(loaded.pull.closed_secs, 600);
        Ok(())
    }

    #[tokio::test]
    async fn missing_enabled_keys_default_to_false() -> Result<(), Error> {
        // Today's behavior: a JSON file missing `pull_enabled` / `push_enabled`
        // loads as `false` rather than erroring. `#[serde(default)]` on the
        // two `enabled` fields preserves that.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let payload = br#"{
            "focused_secs": 30,
            "unfocused_secs": 120,
            "closed_secs": 600,
            "idle_timeout_secs": 30
        }"#;
        tokio::fs::write(&path, payload).await?;
        let loaded = AutosyncSettings::load(dir.path()).await?;
        assert!(!loaded.pull.enabled);
        assert!(!loaded.push.enabled);
        Ok(())
    }
}
