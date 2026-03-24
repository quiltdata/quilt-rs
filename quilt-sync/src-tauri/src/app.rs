use semver::Version;
use tauri::PackageInfo;

use crate::telemetry::prelude::*;
use crate::telemetry::LogsDir;

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

    #[cfg(test)]
    pub fn create() -> crate::Result<Self> {
        Ok(App {
            version: Version::new(0, 0, 999),
            logs_dir: LogsDir::create()?,
        })
    }
}
