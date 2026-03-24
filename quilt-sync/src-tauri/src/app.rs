use mockall::predicate::*;
use mockall::*;
use semver::Version;
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
    fn logs_dir(&self) -> &LogsDir;
}

impl AppAssets for App {
    fn version(&self) -> Version {
        self.version.clone()
    }

    fn logs_dir(&self) -> &LogsDir {
        &self.logs_dir
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
}

#[cfg(test)]
pub mod mocks {
    use std::path::PathBuf;

    use super::*;

    pub fn create() -> MockAppAssets {
        let mut app = MockAppAssets::new();
        app.expect_version()
            .return_const(semver::Version::parse("0.0.999").unwrap());
        app.expect_logs_dir()
            .return_const(LogsDir::Permanent(PathBuf::from("/tmp/quiltsync/logs")));
        app
    }
}
