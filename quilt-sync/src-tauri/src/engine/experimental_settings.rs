use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::engine::error::Error;

const FILE_NAME: &str = "experimental_settings.json";

/// Opt-ins for behaviour that is not finished being designed.
///
/// Persisted as `experimental_settings.json` in `app_local_data_dir` — its own
/// file rather than a key on an existing one, so no experiment leaves a key
/// behind in settings people keep.
///
/// Retiring an experiment needs no migration: unknown fields are ignored on
/// load, and the next save rewrites the file without the retired key.
///
/// Everything here defaults to **off**. An experiment is shown to someone who
/// went looking for it, not shipped quietly to everyone with a switch to turn
/// it back off. The one exception is the dataspace engine's opt-out, which
/// is a way back for one release after the cutover, not an experiment.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ExperimentalSettings {
    /// Whether the Settings page shows the Agent tasks card. Only the card:
    /// the task runner reads `task_settings.json` whatever this says, and
    /// turning it off leaves that file as it is.
    #[serde(default)]
    pub agent_tasks: bool,
    /// Whether the engine keeps to the ref API rather than the dataspace
    /// API, which it runs on since the cutover: see
    /// [`crate::engine::dataspace`]. The opt-out lasts one release, until
    /// the ref API is retired. Read once, when the app starts, so a change
    /// takes effect at the next start. No page sets it: it is for whoever
    /// edits the file.
    ///
    /// Its own key, not a `false` under the `dataspace` key the switch had
    /// before the cutover: every save wrote that key, so a `false` there says
    /// only that the file was saved while the switch was off by default. That
    /// key is now ignored, and dropped on the next save.
    #[serde(default)]
    pub dataspace_opt_out: bool,
}

impl ExperimentalSettings {
    /// Whether the engine runs on the dataspace API: unless the file opts
    /// out.
    pub fn dataspace(&self) -> bool {
        !self.dataspace_opt_out
    }

    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Apply only the flags the caller sent.
    ///
    /// A caller that knows about one experiment must not reset another it has
    /// never heard of — which is what a whole-struct write does the moment a
    /// second flag exists.
    pub fn patch(&mut self, agent_tasks: Option<bool>) {
        if let Some(value) = agent_tasks {
            self.agent_tasks = value;
        }
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

pub type SharedExperimentalSettings = Arc<RwLock<ExperimentalSettings>>;

pub async fn init(data_dir: &Path) -> Result<SharedExperimentalSettings, Error> {
    let settings = ExperimentalSettings::load(data_dir).await?;
    Ok(Arc::new(RwLock::new(settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn roundtrip() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = ExperimentalSettings {
            agent_tasks: true,
            dataspace_opt_out: true,
        };
        settings.save(dir.path()).await?;
        assert_eq!(ExperimentalSettings::load(dir.path()).await?, settings);
        Ok(())
    }

    /// An experiment nobody opted into is off, and a fresh install has no file
    /// at all.
    #[tokio::test]
    async fn missing_file_is_all_off() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = ExperimentalSettings::load(dir.path()).await?;
        assert_eq!(loaded, ExperimentalSettings::default());
        assert!(!loaded.agent_tasks);
        Ok(())
    }

    /// A file written by a build that knew fewer experiments still loads: an
    /// absent key is off, which is the same answer as never having opted in.
    #[tokio::test]
    async fn an_unknown_shape_still_loads() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(dir.path().join(FILE_NAME), b"{}").await?;
        assert!(!ExperimentalSettings::load(dir.path()).await?.agent_tasks);
        Ok(())
    }

    /// Since the cutover the dataspace engine is on unless the file opts
    /// out. The `false` every save wrote under the old `dataspace` key is
    /// not an opt-out.
    #[tokio::test]
    async fn the_dataspace_switch_is_on_unless_the_file_opts_out() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join(FILE_NAME);
        assert!(ExperimentalSettings::load(dir.path()).await?.dataspace());
        for on in [
            r#"{"agent_tasks":true}"#,
            r#"{"agent_tasks":false,"dataspace":false}"#,
            r#"{"dataspace_opt_out":false}"#,
        ] {
            tokio::fs::write(&file, on).await?;
            assert!(
                ExperimentalSettings::load(dir.path()).await?.dataspace(),
                "{on}"
            );
        }
        tokio::fs::write(&file, br#"{"dataspace_opt_out":true}"#).await?;
        assert!(!ExperimentalSettings::load(dir.path()).await?.dataspace());
        Ok(())
    }

    #[test]
    fn patch_leaves_untouched_flags_alone() {
        let mut s = ExperimentalSettings {
            agent_tasks: true,
            dataspace_opt_out: true,
        };
        s.patch(None);
        assert!(
            s.agent_tasks,
            "a flag the caller never sent must not be reset"
        );
        s.patch(Some(false));
        assert!(!s.agent_tasks);
        assert!(
            s.dataspace_opt_out,
            "the switch has no patch, so none resets it"
        );
    }

    /// A retired experiment leaves its key in files written before it went.
    /// Such a file must still load with every live flag intact, and the next
    /// save must drop the key — which is why retiring one is not a migration.
    #[tokio::test]
    async fn a_retired_flag_is_ignored_and_dropped_on_save() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        tokio::fs::write(
            &path,
            br#"{"entire_package_sync":true,"agent_tasks":true,"main_page_v2":true}"#,
        )
        .await?;

        let loaded = ExperimentalSettings::load(dir.path()).await?;
        assert_eq!(
            loaded,
            ExperimentalSettings {
                agent_tasks: true,
                dataspace_opt_out: false,
            }
        );

        loaded.save(dir.path()).await?;
        let written = tokio::fs::read_to_string(&path).await?;
        assert!(
            !written.contains("page_v2") && !written.contains("entire_package_sync"),
            "the retired keys are gone after a save: {written}"
        );
        Ok(())
    }
}
