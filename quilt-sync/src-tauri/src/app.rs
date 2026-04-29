use semver::Version;
use tauri::PackageInfo;

use crate::telemetry::LogsDir;
use crate::telemetry::prelude::*;

pub struct App {
    pub version: Version,
    pub logs_dir: LogsDir,
}

impl App {
    pub fn new(info: &PackageInfo, logs_dir: LogsDir) -> Self {
        debug!("Logs directory is {}", logs_dir.path().display());
        App {
            version: info.version.clone(),
            logs_dir,
        }
    }
}
