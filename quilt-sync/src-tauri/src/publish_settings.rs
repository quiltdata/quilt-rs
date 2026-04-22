use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::Error;

const FILE_NAME: &str = "publish_settings.json";

/// User-configurable defaults for the one-click Publish flow.
///
/// Persisted as `publish_settings.json` in `app_local_data_dir`. All fields
/// are optional — when missing, Publish falls back to `commit_message::generate`
/// and sends no workflow / no metadata.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PublishSettings {
    pub message_template: Option<String>,
    pub default_workflow: Option<String>,
    pub default_metadata: Option<String>,
}

impl PublishSettings {
    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Load settings from disk. Missing file → defaults.
    pub async fn load(data_dir: &Path) -> Result<Self, Error> {
        let path = Self::file_path(data_dir);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(Error::from(err)),
        }
    }

    /// Write atomically: temp file + rename.
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

pub type SharedPublishSettings = Arc<RwLock<PublishSettings>>;

pub async fn init(data_dir: &Path) -> Result<SharedPublishSettings, Error> {
    let settings = PublishSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn roundtrip() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = PublishSettings {
            message_template: Some("Auto-publish {date}".to_string()),
            default_workflow: Some("release".to_string()),
            default_metadata: Some(r#"{"source":"desktop"}"#.to_string()),
        };
        settings.save(dir.path()).await?;
        let loaded = PublishSettings::load(dir.path()).await?;
        assert_eq!(loaded, settings);
        Ok(())
    }

    #[tokio::test]
    async fn missing_file_returns_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = PublishSettings::load(dir.path()).await?;
        assert_eq!(loaded, PublishSettings::default());
        Ok(())
    }
}
