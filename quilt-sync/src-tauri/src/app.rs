use semver::Version;
use tauri::PackageInfo;

use crate::telemetry::prelude::*;
use crate::telemetry::LogsDir;

pub struct App {
    pub version: Version,
    pub logs_dir: LogsDir,
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
impl Default for App {
    fn default() -> Self {
        use std::path::PathBuf;
        App {
            version: Version::new(0, 0, 999),
            logs_dir: LogsDir::Permanent(PathBuf::from("/tmp/quiltsync/logs")),
        }
    }
}
