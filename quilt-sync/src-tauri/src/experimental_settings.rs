use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use quilt_rs::lineage::SyncScope;

use crate::error::Error;

const FILE_NAME: &str = "experimental_settings.json";

/// Opt-ins for behaviour that is not finished being designed.
///
/// Persisted as `experimental_settings.json` in `app_local_data_dir` — its own
/// file rather than a key on an existing one, so retiring an experiment deletes
/// a file instead of leaving a dead key behind in settings people keep.
///
/// Everything here defaults to **off**. An experiment is shown to someone who
/// went looking for it, not shipped quietly to everyone with a switch to turn
/// it back off.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ExperimentalSettings {
    /// Whether the package screen offers a per-package
    /// [sync scope](quilt_rs::lineage::SyncScope) — the choice between
    /// downloading the files you pick and keeping the whole package.
    ///
    /// A **gate on the control**, not on the behaviour: turning this on
    /// downloads nothing by itself, and turning it off leaves the per-package
    /// choices written where they are, so re-enabling resumes them. Clearing
    /// them would destroy a choice the user made in order to undo a setting
    /// that only ever hid a control.
    #[serde(default)]
    pub entire_package_sync: bool,
}

impl ExperimentalSettings {
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

/// The scope to ask the engine for: a package's standing choice, honoured only
/// while the experiment is on.
///
/// quilt-rs takes the scope as an argument and never reads it, so combining the
/// two inputs is this app's job — and it is exactly one rule, in one place, used
/// by both paths that pull. Keeping it a named function rather than an `&&` at
/// each call site is what makes "the tick honours it too" checkable instead of
/// a thing to remember.
#[must_use]
pub fn resolve_sync_scope(stored: SyncScope, settings: &ExperimentalSettings) -> SyncScope {
    if settings.entire_package_sync {
        stored
    } else {
        SyncScope::IndividualFiles
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
            entire_package_sync: true,
        };
        settings.save(dir.path()).await?;
        assert_eq!(ExperimentalSettings::load(dir.path()).await?, settings);
        Ok(())
    }

    /// An experiment nobody opted into is off, and a fresh install has no file
    /// at all — so the package screen is byte-identical to before for everyone
    /// who has not gone looking.
    #[tokio::test]
    async fn missing_file_is_all_off() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let loaded = ExperimentalSettings::load(dir.path()).await?;
        assert_eq!(loaded, ExperimentalSettings::default());
        assert!(!loaded.entire_package_sync);
        Ok(())
    }

    /// A file written by a build that knew fewer experiments still loads: an
    /// absent key is off, which is the same answer as never having opted in.
    #[tokio::test]
    async fn an_unknown_shape_still_loads() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(dir.path().join(FILE_NAME), b"{}").await?;
        assert!(
            !ExperimentalSettings::load(dir.path())
                .await?
                .entire_package_sync
        );
        Ok(())
    }

    fn gate(on: bool) -> ExperimentalSettings {
        ExperimentalSettings {
            entire_package_sync: on,
        }
    }

    /// The gate is a veto, not a switch: it can only ever narrow what a package
    /// asked for. Both cells of the interesting row asserted, so this cannot
    /// pass by the gate being ignored.
    #[test]
    fn the_gate_can_only_narrow_the_stored_scope() {
        assert_eq!(
            resolve_sync_scope(SyncScope::EntirePackage, &gate(true)),
            SyncScope::EntirePackage,
            "on: the package's own choice is honoured"
        );
        assert_eq!(
            resolve_sync_scope(SyncScope::EntirePackage, &gate(false)),
            SyncScope::IndividualFiles,
            "off: the stored choice is ignored, not obeyed"
        );
    }

    /// And it never widens: a package that never opted in is sparse-checkout
    /// under either gate position, so turning the experiment on changes nothing
    /// on its own.
    #[test]
    fn the_gate_never_widens_a_package_that_did_not_ask() {
        for on in [true, false] {
            assert_eq!(
                resolve_sync_scope(SyncScope::IndividualFiles, &gate(on)),
                SyncScope::IndividualFiles,
                "gate on={on}"
            );
        }
    }
}
