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
/// and sends no metadata. A missing or empty `default_workflow` means the
/// bucket's default workflow.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PublishSettings {
    pub message_template: Option<String>,
    pub default_workflow: Option<String>,
    pub default_metadata: Option<String>,
    /// Whether the package page's `Publish` opens the commit page for review
    /// instead of publishing in one click. Off unless the reader turned it on,
    /// and a file written before the setting existed reads as off.
    #[serde(default)]
    pub confirm_before_publish: bool,
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

impl PublishSettings {
    /// These settings with the commit defaults replaced and everything else
    /// kept: the Settings popup edits the defaults and nothing more.
    #[must_use]
    pub fn with_defaults(
        &self,
        message_template: Option<String>,
        default_workflow: Option<String>,
        default_metadata: Option<String>,
    ) -> Self {
        Self {
            message_template,
            default_workflow,
            default_metadata,
            ..self.clone()
        }
    }

    /// These settings with *Confirm before publishing* replaced and the
    /// commit defaults kept: the checkbox edits that one field.
    #[must_use]
    pub fn with_confirm_before_publish(&self, confirm_before_publish: bool) -> Self {
        Self {
            confirm_before_publish,
            ..self.clone()
        }
    }
}

pub type SharedPublishSettings = Arc<RwLock<PublishSettings>>;

/// Change some of the settings, keeping the rest as they are now, and save.
///
/// The change is applied to what is held, under the write lock, rather than
/// to a copy a control read earlier. Two controls edit different fields, and
/// each one sending its whole snapshot back would let a save from one put the
/// other's field back to what it was when the page was drawn.
pub async fn update(
    shared: &SharedPublishSettings,
    data_dir: &Path,
    change: impl FnOnce(&PublishSettings) -> PublishSettings,
) -> Result<(), Error> {
    let mut held = shared.write().await;
    let new = change(&held);
    new.save(data_dir).await?;
    *held = new;
    Ok(())
}

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
            confirm_before_publish: true,
        };
        settings.save(dir.path()).await?;
        let loaded = PublishSettings::load(dir.path()).await?;
        assert_eq!(loaded, settings);
        Ok(())
    }

    /// A file saved before *Confirm before publishing* existed loads, with
    /// the setting off.
    #[tokio::test]
    async fn a_file_without_confirm_before_publish_reads_as_off() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(
            dir.path().join(FILE_NAME),
            br#"{"message_template":"Auto-publish {date}","default_workflow":null,"default_metadata":null}"#,
        )
        .await?;
        let loaded = PublishSettings::load(dir.path()).await?;
        assert_eq!(
            loaded.message_template.as_deref(),
            Some("Auto-publish {date}")
        );
        assert!(!loaded.confirm_before_publish);
        Ok(())
    }

    fn shared(settings: PublishSettings) -> SharedPublishSettings {
        Arc::new(RwLock::new(settings))
    }

    /// Saving the commit defaults leaves *Confirm before publishing* on, in
    /// memory and on disk.
    #[tokio::test]
    async fn saving_the_defaults_keeps_confirm_before_publish() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let held = shared(PublishSettings {
            confirm_before_publish: true,
            ..PublishSettings::default()
        });
        update(&held, dir.path(), |current| {
            current.with_defaults(Some("Nightly {date}".to_string()), None, None)
        })
        .await?;
        let expected = PublishSettings {
            message_template: Some("Nightly {date}".to_string()),
            confirm_before_publish: true,
            ..PublishSettings::default()
        };
        assert_eq!(*held.read().await, expected);
        assert_eq!(PublishSettings::load(dir.path()).await?, expected);
        Ok(())
    }

    /// Turning *Confirm before publishing* on leaves the commit defaults as
    /// they are, in memory and on disk.
    #[tokio::test]
    async fn saving_confirm_before_publish_keeps_the_defaults() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let defaults = PublishSettings {
            message_template: Some("Nightly {date}".to_string()),
            default_workflow: Some("release".to_string()),
            default_metadata: Some(r#"{"source":"desktop"}"#.to_string()),
            confirm_before_publish: false,
        };
        let held = shared(defaults.clone());
        update(&held, dir.path(), |current| {
            current.with_confirm_before_publish(true)
        })
        .await?;
        let expected = PublishSettings {
            confirm_before_publish: true,
            ..defaults
        };
        assert_eq!(*held.read().await, expected);
        assert_eq!(PublishSettings::load(dir.path()).await?, expected);
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
