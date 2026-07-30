use std::sync::{Arc, Mutex};

use ::sentry as Sentry;
use mixpanel_rs::Mixpanel;
use quilt_uri::Host;
use semver::Version;

use crate::Result;

pub mod diagnostics;
pub mod event;
pub mod mixpanel;
pub mod sentry;
pub mod tracing;

pub use event::MixpanelEvent;
pub use tracing::LogsDir;

pub mod prelude {
    pub use tracing::{debug, error, info, warn};
}

/// The deployment the app was last seen working with, shared with the Sentry
/// client's event hook.
///
/// A crash's capture site is not ours — the panic hook fires wherever it fires,
/// and error paths know nothing of hosts — so the host a crash reports has to
/// be *ambient*. Tagging the Sentry scope cannot provide that ambience: hubs
/// are thread-local and a worker's hub is a snapshot of the process scope taken
/// when that thread first touched Sentry, so a tag set inside a command handler
/// reaches crashes on that one worker and nowhere else. The hook reads this cell
/// instead, on whatever thread the event leaves from.
pub type AmbientHost = Arc<Mutex<Option<Host>>>;

pub struct Telemetry {
    _sentry: Option<Sentry::ClientInitGuard>,
    mixpanel: Option<Arc<Mixpanel>>,
    /// The host every outgoing crash report is tagged with; see [`AmbientHost`].
    host: AmbientHost,
}

impl Telemetry {
    pub fn new(version: &Version, enable: Option<()>) -> Self {
        // The cell outlives the client and is read by its event hook, so it has
        // to exist before the client is built.
        let host: AmbientHost = Arc::new(Mutex::new(None));

        Self {
            mixpanel: enable
                .and(mixpanel::mixpanel_config())
                .map(|(token, config)| Arc::new(Mixpanel::init(&token, Some(config)))),
            _sentry: enable
                .and(sentry::sentry_config(version, Arc::clone(&host)))
                .map(::sentry::init),
            host,
        }
    }

    pub fn init_file_logging(base_path: &std::path::Path) -> Result<LogsDir> {
        tracing::init_file_logging(base_path)
    }

    /// Record `host` as the deployment in play, for crash reports.
    ///
    /// Latest wins: a crash reports the deployment of the most recent action,
    /// which is the useful answer when a session works more than one stack.
    /// Read paths that learn a host call this directly; every host-bearing
    /// [`Self::track`] call routes through it too.
    pub fn add_host(&self, host: &Host) {
        if let Ok(mut current) = self.host.lock() {
            *current = Some(host.clone());
        }
    }

    /// Emit `event`, attributed to the deployment its payload names.
    ///
    /// Which events can name one is a property of each event's payload type, so
    /// there is nothing to pass here: an app-lifecycle or debug event has no
    /// host to give, and a deployment-bearing one carries it already.
    pub async fn track(&self, event: MixpanelEvent) {
        if let Some(host) = event.host() {
            self.add_host(host);
        }
        if let Err(err) = mixpanel::track_event(self.mixpanel.as_ref(), &event).await {
            Sentry::capture_error(&err);
        }
    }

    /// Report an anomaly that produced no error value to carry — something that
    /// should not have happened but that no `Err` describes.
    ///
    /// Keep `message` a constant: the crash reporter groups by it, so the
    /// variable part belongs in the host tag ([`Self::add_host`]) rather than in
    /// the text, or one anomaly becomes one issue per host.
    pub fn report_anomaly(message: &str) {
        Sentry::capture_message(message, Sentry::Level::Warning);
    }

    /// Report a fault to the crash reporter without failing the caller.
    ///
    /// For the case an event cannot describe: an emitter that should be able to
    /// name its deployment and cannot. Reporting it as a fault keeps the
    /// analytics vocabulary free of error events, which would need their own
    /// design for what an error is allowed to say.
    pub fn report_error(err: &(dyn std::error::Error + Send + Sync + 'static)) {
        Sentry::capture_error(err);
    }

    pub fn init(&self) {
        mixpanel::init(self.mixpanel.as_ref());
    }

    /// Returns the current global maximum log level as a human-readable string.
    pub fn log_level() -> String {
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
