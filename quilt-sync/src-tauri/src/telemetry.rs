use std::sync::{Arc, Mutex};

use ::sentry as Sentry;
use quilt_uri::Host;
use semver::Version;

use crate::Result;

pub mod diagnostics;
pub mod event;
pub mod mixpanel;
pub mod sentry;
pub mod tracing;

pub use event::MixpanelEvent;
pub use mixpanel::Analytics;
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

/// Whether the sinks are live, and which build of *ourselves* they report as.
///
/// A release build reports as production against the credentials baked in at
/// build time. A local build is silent unless a developer opts in
/// ([`env::telemetry_dev_opt_in`]), and when it does it reports separately, so
/// this traffic is never counted as a user's.
///
/// Two things keep the separation structural rather than conventional. The
/// opt-in has no build-time form, so no released binary can carry it; and a
/// local build's credentials can only come from the `.env` that
/// [`env::init`](crate::env::init) loads under `debug_assertions`, so pointing a
/// dev build at the real sinks takes a deliberate act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sinks {
    /// Release build: the sinks configured at build time.
    Production,
    /// Local build with an explicit opt-in: the sinks a developer configured.
    Development,
    Disabled,
}

impl Sinks {
    /// Decide from the build profile and the developer's opt-in.
    ///
    /// The invariant worth protecting: a developer who does nothing gets
    /// `Disabled`. Enabling telemetry locally is always an act, never a default.
    pub fn resolve() -> Self {
        if cfg!(debug_assertions) {
            if crate::env::telemetry_dev_opt_in() {
                Self::Development
            } else {
                Self::Disabled
            }
        } else {
            Self::Production
        }
    }

    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// The `environment` an outgoing report is attributed to, so internal
    /// traffic is filterable at the sink rather than only by convention.
    ///
    /// Only the crash sink uses this. Analytics needs no equivalent, because a
    /// local build sends nothing at all — see [`mixpanel::Analytics`].
    pub fn environment(self) -> Option<&'static str> {
        match self {
            Self::Production => Some("production"),
            Self::Development => Some("development"),
            Self::Disabled => None,
        }
    }
}

pub struct Telemetry {
    _sentry: Option<Sentry::ClientInitGuard>,
    analytics: Analytics,
    /// The host every outgoing crash report is tagged with; see [`AmbientHost`].
    host: AmbientHost,
}

impl Telemetry {
    pub fn new(version: &Version, sinks: Sinks) -> Self {
        // The cell outlives the client and is read by its event hook, so it has
        // to exist before the client is built.
        let host: AmbientHost = Arc::new(Mutex::new(None));

        Self {
            analytics: Analytics::resolve(sinks),
            // The crash sink has no dry run — the SDK either holds a client or it
            // does not — so a local build reaches it only if a developer supplies
            // a DSN, and then reports as `development`. That is deliberate: it
            // keeps crash-side behaviour testable by someone who has a spare
            // project, without requiring one.
            _sentry: sinks
                .is_enabled()
                .then(|| sentry::sentry_config(version, sinks, Arc::clone(&host)))
                .flatten()
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
        if let Err(err) = mixpanel::track_event(&self.analytics, &event).await {
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
        mixpanel::init(&self.analytics);
    }

    /// Returns the current global maximum log level as a human-readable string.
    pub fn log_level() -> String {
        ::tracing::level_filters::LevelFilter::current().to_string()
    }
}

#[cfg(test)]
impl Default for Telemetry {
    fn default() -> Self {
        // Tests never build a client: `Disabled` short-circuits before any
        // credential is read, so a test cannot emit even if the environment
        // happens to carry a token.
        let version = semver::Version::new(0, 0, 0);
        Self::new(&version, Sinks::Disabled)
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    /// Tests build with `debug_assertions` on, so [`Sinks::resolve`] takes its
    /// local-build branch here — which is the branch worth pinning: the opt-in
    /// is the only thing between a developer and live telemetry.
    ///
    /// `#[serial]` because these mutate the process environment, which races any
    /// concurrent reader — the same reason [`crate::env`]'s tests are serial.
    #[test]
    #[serial]
    fn a_local_build_is_silent_until_it_opts_in() {
        unsafe {
            std::env::remove_var("QUILTSYNC_TELEMETRY_DEV");
        }
        assert_eq!(
            Sinks::resolve(),
            Sinks::Disabled,
            "a developer who sets nothing must get no telemetry"
        );

        unsafe {
            std::env::set_var("QUILTSYNC_TELEMETRY_DEV", "1");
        }
        assert_eq!(Sinks::resolve(), Sinks::Development);

        // `0` is the one value that reads as "no", so the opt-in can be left in
        // a `.env` and switched off without deleting the line.
        unsafe {
            std::env::set_var("QUILTSYNC_TELEMETRY_DEV", "0");
        }
        assert_eq!(Sinks::resolve(), Sinks::Disabled);

        unsafe {
            std::env::remove_var("QUILTSYNC_TELEMETRY_DEV");
        }
    }

    /// The last link before the network: an opted-in build with a token really
    /// does construct an analytics client. Without this, every other test here
    /// could pass while the rig emitted nothing.
    ///
    /// `SENTRY_DSN` is deliberately left unset — initializing the crash client
    /// would spawn a real transport thread, and the analytics half is what proves
    /// the wiring.
    /// An opted-in local build reaches the dry run — and reaches it *even with a
    /// token present*, which is the property that makes the isolation
    /// structural. A developer holding real credentials still cannot emit from
    /// their laptop.
    #[test]
    #[serial]
    fn an_opted_in_build_dry_runs_even_holding_a_token() {
        unsafe {
            std::env::set_var("QUILTSYNC_TELEMETRY_DEV", "1");
            std::env::set_var("MIXPANEL_PROJECT_TOKEN", "a-token-that-must-not-be-used");
            std::env::remove_var("SENTRY_DSN");
        }

        let telemetry = Telemetry::new(&Version::new(0, 0, 0), Sinks::resolve());
        assert!(
            matches!(telemetry.analytics, Analytics::DryRun),
            "a local build must never construct a live client, token or not"
        );

        unsafe {
            std::env::remove_var("QUILTSYNC_TELEMETRY_DEV");
            std::env::remove_var("MIXPANEL_PROJECT_TOKEN");
        }
    }

    /// An empty value is not an opt-in: `get_var` treats empty as unset, so
    /// `QUILTSYNC_TELEMETRY_DEV=` in a `.env` stays silent rather than enabling
    /// telemetry on a blank line.
    #[test]
    #[serial]
    fn an_empty_opt_in_is_not_an_opt_in() {
        unsafe {
            std::env::set_var("QUILTSYNC_TELEMETRY_DEV", "");
        }
        assert_eq!(Sinks::resolve(), Sinks::Disabled);
        unsafe {
            std::env::remove_var("QUILTSYNC_TELEMETRY_DEV");
        }
    }

    /// The invariant this module exists to protect: enabling telemetry on a
    /// local build is an act. `resolve` is compiled per profile, so this asserts
    /// the branch that actually shipped rather than both.
    #[test]
    #[allow(
        clippy::used_underscore_binding,
        reason = "`_sentry` is underscore-prefixed because it is held only for its Drop; \
                  whether it was constructed at all is exactly what this test asserts"
    )]
    fn disabled_sinks_build_no_client() {
        let telemetry = Telemetry::new(&Version::new(0, 0, 0), Sinks::Disabled);
        assert!(matches!(telemetry.analytics, Analytics::Off));
        assert!(telemetry._sentry.is_none());
    }

    #[test]
    fn only_disabled_is_silent() {
        assert!(!Sinks::Disabled.is_enabled());
        assert!(Sinks::Production.is_enabled());
        assert!(Sinks::Development.is_enabled());
    }

    /// A silent build reports no environment, and the two live ones never share
    /// one — that separation is the whole point of the variant split.
    #[test]
    fn live_sinks_report_distinct_environments() {
        assert_eq!(Sinks::Disabled.environment(), None);
        assert_eq!(Sinks::Production.environment(), Some("production"));
        assert_eq!(Sinks::Development.environment(), Some("development"));
    }
}
