use std::sync::{Arc, Mutex};

use ::sentry as Sentry;
use mixpanel_rs::Mixpanel;
use quilt_uri::{Host, S3PackageUri};
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

/// What an event concerns, beyond the event itself.
///
/// A struct rather than a bare host so the next dimension we need is added in
/// one place: new context fields go here, not onto individual
/// [`MixpanelEvent`] variants, which keeps a property spelled one way across
/// every event that carries it. [`Self::default`] is the honest context for an
/// event that concerns nothing in particular.
#[derive(Debug, Clone, Default)]
pub struct EventContext {
    /// The Quilt deployment the event concerns.
    ///
    /// `None` means it concerns none — the event then carries no host rather
    /// than inheriting the last one seen, so a host in the data is always the
    /// deployment the user acted on.
    pub host: Option<Host>,
}

impl EventContext {
    /// Context for an event concerning `host`.
    pub fn for_host(host: Option<&Host>) -> Self {
        Self {
            host: host.cloned(),
        }
    }

    /// Context for an event concerning a package, taken from the URI the acting
    /// surface rendered from. `None` catalog (or no URI) means the package has
    /// no remote yet, so the event concerns no deployment.
    pub fn for_uri(uri: Option<&S3PackageUri>) -> Self {
        Self::for_host(uri.and_then(|uri| uri.catalog.as_ref()))
    }
}

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

    /// Emit `event`, attributed by `context` to what it concerns.
    ///
    /// [`EventContext::default`] is for an event that concerns no deployment at
    /// all — app launch, first-run setup, the local debug and diagnostics
    /// actions — which then carries no host rather than inheriting one.
    pub async fn track(&self, event: MixpanelEvent, context: EventContext) {
        if let Some(host) = &context.host {
            self.add_host(host);
        }
        if let Err(err) = mixpanel::track_event(self.mixpanel.as_ref(), &event, &context).await {
            Sentry::capture_error(&err);
        }
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
