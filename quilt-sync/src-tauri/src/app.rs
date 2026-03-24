use mockall::predicate::*;
use mockall::*;
use semver::Version;
use std::path::PathBuf;
use tauri::PackageInfo;

use crate::telemetry::prelude::*;
use crate::telemetry::LogsDir;

pub struct App {
    version: Version,
    logs_dir: LogsDir,
}

#[automock]
pub trait AppAssets {
    fn version(&self) -> Version;
    fn logs_dir_path(&self) -> PathBuf;
    fn logs_dir_is_temporary(&self) -> bool;
}

impl AppAssets for App {
    fn version(&self) -> Version {
        self.version.clone()
    }

    fn logs_dir_path(&self) -> PathBuf {
        self.logs_dir.path().to_path_buf()
    }

    fn logs_dir_is_temporary(&self) -> bool {
        matches!(self.logs_dir, LogsDir::Temporary(_))
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
pub mod mocks {
    use super::*;

    pub fn create() -> MockAppAssets {
        let mut app = MockAppAssets::new();
        app.expect_version()
            .return_const(semver::Version::parse("0.0.999").unwrap());
        app.expect_logs_dir_path()
            .return_const(PathBuf::from("/tmp/quiltsync/logs"));
        app.expect_logs_dir_is_temporary().return_const(false);
        app
    }
}
