use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use ::sentry as Sentry;
use mixpanel_rs::Mixpanel;
use semver::Version;

use crate::Result;

pub mod diagnostics;
pub mod mixpanel;
pub mod sentry;
pub mod tracing;

pub use mixpanel::MixpanelEvent;
pub use tracing::LogsDir;

pub mod prelude {
    pub use tracing::{debug, error, info, warn};
}

pub struct Telemetry {
    _sentry: Option<Sentry::ClientInitGuard>,
    mixpanel: Option<Arc<Mixpanel>>,
    hosts: Mutex<BTreeSet<String>>,
}

impl Telemetry {
    pub fn new(version: &Version, enable: Option<()>) -> Self {
        Self {
            mixpanel: enable
                .and(mixpanel::mixpanel_config())
                .map(|(token, config)| Arc::new(Mixpanel::init(&token, Some(config)))),
            _sentry: enable
                .and(sentry::sentry_config(version))
                .map(::sentry::init),
            hosts: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn init_file_logging(&self, base_path: &std::path::Path) -> Result<LogsDir> {
        tracing::init_file_logging(base_path)
    }

    pub fn add_host(&self, host: &quilt_uri::Host) {
        if let Ok(mut hosts) = self.hosts.lock() {
            if hosts.insert(host.to_string()) {
                Sentry::configure_scope(|scope| {
                    scope.set_tag("quilt_host", host.to_string());
                });
            }
        }
    }

    pub async fn track(&self, event: MixpanelEvent) {
        if let Err(err) = mixpanel::track_event(&self.mixpanel, &event).await {
            Sentry::capture_error(&err);
        }
    }

    pub fn init(&self) {
        mixpanel::init(&self.mixpanel);
    }

    /// Returns the current global maximum log level as a human-readable string.
    pub fn log_level(&self) -> String {
        ::tracing::level_filters::LevelFilter::current().to_string()
    }
}

#[cfg(test)]
impl Default for Telemetry {
    fn default() -> Self {
        // In tests, use non-production mode (no telemetry)
        let version = semver::Version::new(0, 0, 0);
        Self::new(&version, None)
    }
}
