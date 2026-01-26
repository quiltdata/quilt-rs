use mockall::predicate::*;
use mockall::*;
use semver::Version;
use std::path::PathBuf;
use tauri::PackageInfo;

use crate::telemetry::prelude::*;
use crate::telemetry::LogsDir;

#[derive(Debug, Clone)]
pub struct Globals {
    pub version: Version,
    pub logs_dir: PathBuf,
    pub logs_dir_is_temporary: bool,
}

pub struct App {
    version: Version,
    logs_dir: LogsDir,
}

#[automock]
pub trait AppAssets {
    fn globals(&self) -> Globals;
}

impl AppAssets for App {
    fn globals(&self) -> Globals {
        Globals {
            version: self.version.clone(),
            logs_dir: self.logs_dir.path().to_path_buf(),
            logs_dir_is_temporary: matches!(self.logs_dir, LogsDir::Temporary(_)),
        }
    }
}

impl App {
    pub fn create(info: &PackageInfo, logs_dir: LogsDir) -> Self {
        debug!("Logs directory is {}", logs_dir.path().display());
        App {
            version: info.version.clone(),
            logs_dir,
        }
    }

    pub fn logs_dir(&self) -> &LogsDir {
        &self.logs_dir
    }
}

#[cfg(test)]
impl Default for Globals {
    fn default() -> Self {
        Globals {
            version: Version::new(0, 0, 0),
            logs_dir: PathBuf::from("/tmp/quiltsync/logs"),
            logs_dir_is_temporary: false,
        }
    }
}

#[cfg(test)]
pub mod mocks {
    use super::*;

    pub fn create() -> MockAppAssets {
        let mut app = MockAppAssets::new();
        app.expect_globals().return_const(Globals {
            version: semver::Version::parse("0.0.999").unwrap(),
            logs_dir: PathBuf::from("/tmp/quiltsync/logs"),
            logs_dir_is_temporary: false,
        });
        app
    }
}
