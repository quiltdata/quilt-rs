use semver::Version;
use tauri::PackageInfo;

use crate::telemetry::Logging;
use crate::telemetry::prelude::*;

pub struct App {
    pub version: Version,
    /// Where the log is written, and the worker that writes it.
    ///
    /// Held here for the life of the app because dropping it stops the writer —
    /// the log would end quietly at startup, which is the failure mode a
    /// background writer trades for not blocking the emitting thread.
    pub logging: Logging,
}

impl App {
    pub fn new(info: &PackageInfo, logging: Logging) -> Self {
        debug!("Logs directory is {}", logging.dir.path().display());
        App {
            version: info.version.clone(),
            logging,
        }
    }
}
