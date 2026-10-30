use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::error::Error;

const FILE_NAME: &str = "design_settings.json";

/// Which design generation the app draws.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Design {
    /// The classic design, for a reader who chose it in Settings.
    V1,
    #[default]
    V2,
}

/// The reader's design generation.
///
/// Persisted as `design_settings.json` in `app_local_data_dir`, and read on
/// every Settings read, so it holds no state in the app. A file of its
/// own rather than a key among the experiments: the old preview flag there
/// defaulted to off and was saved with every other experiment, so a stored
/// `false` could not tell a reader who chose v1 from one who never looked.
/// This file starts empty for everyone, so everyone starts on v2.
///
/// Removing v1 deletes this file's module along with the pages it chooses
/// between.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DesignSettings {
    #[serde(default)]
    pub design: Design,
}

impl DesignSettings {
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

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::experimental_settings::ExperimentalSettings;

    #[tokio::test]
    async fn missing_file_is_v2() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        assert_eq!(DesignSettings::load(dir.path()).await?.design, Design::V2);
        Ok(())
    }

    /// A reader whose experiments file says the old preview was off still
    /// opens in v2: that flag is no longer read.
    #[tokio::test]
    async fn an_old_preview_off_still_loads_as_v2() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(
            dir.path().join("experimental_settings.json"),
            br#"{"entire_package_sync":true,"main_page_v2":false}"#,
        )
        .await?;

        assert!(
            ExperimentalSettings::load(dir.path())
                .await?
                .entire_package_sync
        );
        assert_eq!(DesignSettings::load(dir.path()).await?.design, Design::V2);
        Ok(())
    }

    #[tokio::test]
    async fn design_round_trips() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        for design in [Design::V1, Design::V2] {
            DesignSettings { design }.save(dir.path()).await?;
            assert_eq!(DesignSettings::load(dir.path()).await?.design, design);
        }
        let written = tokio::fs::read_to_string(dir.path().join(FILE_NAME)).await?;
        assert!(written.contains(r#""design": "v2""#), "{written}");
        Ok(())
    }

    #[tokio::test]
    async fn a_file_without_the_key_is_v2() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(dir.path().join(FILE_NAME), b"{}").await?;
        assert_eq!(DesignSettings::load(dir.path()).await?.design, Design::V2);
        Ok(())
    }
}
