use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use ::sentry as Sentry;
use quilt_uri::Host;
use semver::Version;

use crate::Result;
use crate::telemetry::prelude::*;

pub mod diagnostics;
pub mod event;
pub mod install_id;
pub mod mixpanel;
pub mod sentry;
pub mod tracing;

pub use event::MixpanelEvent;
pub use install_id::InstallId;
pub use mixpanel::Analytics;
pub use sentry::Faults;
pub use tracing::{Logging, LogsDir};

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

/// Where this build's telemetry goes — decided by the build profile alone.
///
/// **Nothing leaves a local build.** Analytics dry-runs to the terminal (see
/// [`mixpanel::Analytics`]) and the crash reporter is not constructed at all, so
/// no configuration, environment variable, or stray `.env` credential can make a
/// developer's machine emit. That is why this needs no opt-in: an opt-in guards a
/// risk, and there is none left to guard.
///
/// The crash reporter is the asymmetric half. It has no dry run — the SDK either
/// holds a client or it does not — so rather than gate it on a flag, a local build
/// simply never builds one. The cost is that crash-side behaviour (breadcrumbs,
/// stack traces, release health) stays release-verified; the alternative was a
/// `.env` DSN quietly shipping a developer's crashes, which is the thing a
/// deliberate act was supposed to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sinks {
    /// Release build: the sinks configured at build time.
    Production,
    /// Local build: analytics dry-runs, the crash reporter is absent.
    Development,
}

impl Sinks {
    pub fn resolve() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Production
        }
    }

    /// Whether a crash client is built. True only for a release build — see the
    /// type's note on why a local build never reports crashes.
    pub fn reports_crashes(self) -> bool {
        matches!(self, Self::Production)
    }
}

pub struct Telemetry {
    _sentry: Option<Sentry::ClientInitGuard>,
    analytics: Analytics,
    /// The host every outgoing crash report is tagged with; see [`AmbientHost`].
    host: AmbientHost,
    /// This install's identity, on every analytics event and every crash report
    /// so the two can be read together. `None` when it could not be persisted —
    /// see [`InstallId::load`].
    install_id: Option<InstallId>,
    /// Where a fault goes. A field rather than a direct call to the crash SDK, so
    /// that "this path reported a fault" is observable — see [`Faults`].
    faults: Faults,
    /// Whether a refused event has already been reported this run.
    ///
    /// A refusal is systematic — a bad token, a property the API will not take —
    /// so it fails every event until someone fixes it. Reporting each would turn
    /// one misconfiguration into one crash report per user action, and the first
    /// carries the whole signal.
    ///
    /// Shared rather than owned, because the background sender is where failures
    /// are now discovered and it holds its own handle.
    refusal_reported: Arc<AtomicBool>,
    /// Hands an event to the sender.
    ///
    /// `mpsc` is multi-producer, single-consumer: **many** callers push — any
    /// command handler, on any thread, plus the autosync loop — and exactly **one**
    /// task pops, which is why requests never overlap and order survives.
    ///
    /// Two properties this relies on. Pushing is *synchronous*, which is what lets
    /// [`Self::track`] be sync at all. And the channel is *bounded*, so `try_send`
    /// fails at once when full instead of blocking: a stuck sender must never
    /// become a stalled click, nor unbounded memory.
    ///
    /// **Accepted is not delivered.** The queue is in memory and the sender is a
    /// detached task, so events still waiting when the process exits are lost —
    /// in practice the last action or two, since an idle app drains almost
    /// immediately. Deliberately not solved with a flush on exit: that would block
    /// quit on a network call to recover very little, and a queue persisted to disk
    /// answers both this and an offline session with one mechanism.
    queue: mpsc::Sender<mixpanel::Queued>,
    /// The single consumer's half, until [`Self::init`] hands it to that task.
    ///
    /// `Option` because it is given away exactly once — there is only one consumer,
    /// so taking it is what makes a second sender impossible rather than merely
    /// unwise.
    ///
    /// Held here rather than spawned in the constructor because the async runtime
    /// is not up yet at that point — the same reason app launch was already
    /// reported from `init`. Events pushed before then wait in the channel rather
    /// than being lost.
    pending: Mutex<Option<mpsc::Receiver<mixpanel::Queued>>>,
}

/// Report a failed send, or decline to.
///
/// A free function because both the caller's thread and the background sender need
/// it, and the sender owns its own clones of what it reads.
fn report_delivery_failure(faults: &Faults, refusal_reported: &AtomicBool, err: &crate::Error) {
    match mixpanel::DeliveryFailure::classify(err) {
        mixpanel::DeliveryFailure::Refused => {
            if !refusal_reported.swap(true, Ordering::Relaxed) {
                faults.error(err);
            }
        }
        mixpanel::DeliveryFailure::Unreachable => {
            warn!("telemetry: event not delivered: {err}");
        }
    }
}

impl Telemetry {
    pub fn new(version: &Version, sinks: Sinks, install_id: Option<InstallId>) -> Self {
        // The cell outlives the client and is read by its event hook, so it has
        // to exist before the client is built.
        let host: AmbientHost = Arc::new(Mutex::new(None));

        let (queue, pending) = mpsc::channel(mixpanel::QUEUE_DEPTH);

        Self {
            analytics: Analytics::resolve(sinks),
            _sentry: sinks
                .reports_crashes()
                .then(|| sentry::sentry_config(version, install_id.clone(), Arc::clone(&host)))
                .flatten()
                .map(::sentry::init),
            host,
            install_id,
            faults: Faults::resolve(sinks),
            refusal_reported: Arc::new(AtomicBool::new(false)),
            queue,
            pending: Mutex::new(Some(pending)),
        }
    }

    /// This install's identity, for the diagnostic export — so a report a user
    /// emails in can be lined up against that install's event stream.
    pub fn install_id(&self) -> Option<&InstallId> {
        self.install_id.as_ref()
    }

    pub fn init_file_logging(base_path: &std::path::Path) -> Result<Logging> {
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
    ///
    /// **Synchronous, and that is the point.** Handing an event to a queue cannot
    /// fail slowly, so no caller waits on a network call to finish its own work —
    /// and a caller that is itself synchronous can emit at all, which is what lets
    /// the outcome of a command be reported from the one place that knows it.
    pub fn track(&self, event: MixpanelEvent) {
        let queued = mixpanel::Queued::now(event);
        if let Some(host) = queued.host() {
            self.add_host(host);
        }
        // A full queue means the sender is not draining, which is already the
        // problem; dropping the newest event is the cheapest way to stay bounded,
        // and it is logged rather than reported because it is a symptom.
        if self.queue.try_send(queued).is_err() {
            warn!("telemetry: queue full or closed, event dropped");
        }
    }

    /// Report an anomaly that produced no error value to carry — something that
    /// should not have happened but that no `Err` describes.
    ///
    /// Keep `message` a constant: the crash reporter groups by it, so the
    /// variable part belongs in the host tag ([`Self::add_host`]) rather than in
    /// the text, or one anomaly becomes one issue per host.
    pub fn report_anomaly(&self, message: &str) {
        self.faults.anomaly(message);
    }

    /// Report a fault to the crash reporter without failing the caller.
    ///
    /// For the case an event cannot describe: an emitter that should be able to
    /// name its deployment and cannot. Reporting it as a fault keeps the
    /// analytics vocabulary free of error events, which would need their own
    /// design for what an error is allowed to say.
    pub fn report_error(&self, err: &(dyn std::error::Error + Send + Sync + 'static)) {
        self.faults.error(err);
    }

    /// The sender's failure decision, reachable from a test without a runtime.
    #[cfg(test)]
    fn report_delivery_failure_for_test(&self, err: &crate::Error) {
        report_delivery_failure(&self.faults, &self.refusal_reported, err);
    }

    /// The names of the events sitting in the queue, in order.
    ///
    /// Reads the channel rather than a recording of it, so a test observes what a
    /// caller actually handed over — not a parallel bookkeeping that could drift
    /// from it. Draining is fine: nothing consumes the queue in a test, because
    /// the sender is only started by [`Self::init`].
    #[cfg(test)]
    pub fn queued_events(&self) -> Vec<String> {
        let Ok(mut pending) = self.pending.lock() else {
            return Vec::new();
        };
        let Some(receiver) = pending.as_mut() else {
            return Vec::new();
        };

        let mut names = Vec::new();
        while let Ok(queued) = receiver.try_recv() {
            names.push(queued.name());
        }
        names
    }

    /// What was reported through [`Self::report_anomaly`] and
    /// [`Self::report_error`] — the seam that made this unit worth doing. Without
    /// it a path can claim to report a fault and no test can tell.
    #[cfg(test)]
    pub fn reported_faults(&self) -> Vec<String> {
        self.faults.reported()
    }

    /// Start the sender, then report the launch.
    ///
    /// Separate from construction because the async runtime is not up yet at that
    /// point. Until this runs, events queue rather than being lost — which is why
    /// app launch can be reported here despite being the earliest thing that
    /// happens.
    pub fn init(&self) {
        let Some(queue) = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
        else {
            error!("telemetry: sender already started");
            return;
        };

        let analytics = self.analytics.clone();
        let install_id = self.install_id.clone();
        let faults = self.faults.clone();
        let refusal_reported = Arc::clone(&self.refusal_reported);

        tauri::async_runtime::spawn(async move {
            mixpanel::run_sender(analytics, install_id, queue, move |err| {
                report_delivery_failure(&faults, &refusal_reported, err);
            })
            .await;
        });

        self.track(MixpanelEvent::AppLaunched);
    }

    /// Returns the current global maximum log level as a human-readable string.
    pub fn log_level() -> String {
        ::tracing::level_filters::LevelFilter::current().to_string()
    }
}

#[cfg(test)]
impl Default for Telemetry {
    /// A test double, built as one rather than resolved from a build mode: a test
    /// wants a `Telemetry` that neither sends nor prints, which is not something
    /// any shipped build is. Constructing the fields directly keeps [`Sinks`]
    /// describing only the two builds that exist.
    fn default() -> Self {
        let (queue, pending) = mpsc::channel(mixpanel::QUEUE_DEPTH);
        Self {
            _sentry: None,
            analytics: Analytics::Off,
            host: Arc::new(Mutex::new(None)),
            install_id: None,
            // Recording rather than printing, so a test can assert what a path
            // reported instead of a developer reading it go by.
            faults: Faults::Recorded(Arc::new(Mutex::new(Vec::new()))),
            refusal_reported: Arc::new(AtomicBool::new(false)),
            queue,
            pending: Mutex::new(Some(pending)),
        }
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    fn refusal() -> crate::Error {
        crate::Error::from(mixpanel_rs::error::Error::ApiClientError(
            400,
            "no such token".to_owned(),
        ))
    }

    /// A refusal is reported, because it is our bug and fails every event alike.
    /// Reported **once**, because the second report says nothing the first did not
    /// — and without that, one bad token becomes one crash report per user action.
    #[test]
    fn a_refusal_is_reported_once_however_often_it_recurs() {
        let telemetry = Telemetry::default();

        for _ in 0..5 {
            telemetry.report_delivery_failure_for_test(&refusal());
        }

        let reported = telemetry.reported_faults();
        assert_eq!(
            reported.len(),
            1,
            "one misconfiguration is one report: {reported:?}"
        );
        assert!(reported[0].contains("no such token"), "{reported:?}");
    }

    /// An unreachable API is nobody's bug. Reporting it would fill the crash
    /// reporter with other people's flaky connections and bury the refusals.
    #[test]
    fn an_unreachable_api_is_never_reported() {
        let telemetry = Telemetry::default();

        telemetry.report_delivery_failure_for_test(&crate::Error::from(
            mixpanel_rs::error::Error::MaxRetriesReached("gave up".to_owned()),
        ));

        assert!(
            telemetry.reported_faults().is_empty(),
            "a bad connection is not a fault to report: {:?}",
            telemetry.reported_faults()
        );
    }

    /// And a network failure must not consume the one refusal report — otherwise
    /// the first flaky tunnel masks a real misconfiguration for the whole run.
    ///
    /// Both kinds of not-reaching-the-API are exercised, because this exact property
    /// regressed once through the kind that was missing: a send timeout borrowed the
    /// serialization error, classified as a refusal, and spent the report.
    #[test]
    fn a_network_failure_does_not_spend_the_refusal_report() {
        for network in [
            crate::Error::from(mixpanel_rs::error::Error::ApiServerError(503)),
            crate::Error::from(crate::error::TelemetryError::SendTimeout(10)),
        ] {
            let telemetry = Telemetry::default();

            telemetry.report_delivery_failure_for_test(&network);
            assert!(
                telemetry.reported_faults().is_empty(),
                "{network} is not a fault to report"
            );

            telemetry.report_delivery_failure_for_test(&refusal());
            assert_eq!(
                telemetry.reported_faults().len(),
                1,
                "the refusal still got through after {network}"
            );
        }
    }

    /// Tests build with `debug_assertions` on, so this pins the branch a
    /// developer actually runs.
    #[test]
    fn a_local_build_resolves_to_development() {
        assert_eq!(Sinks::resolve(), Sinks::Development);
    }

    /// The invariant, and the reason no opt-in is needed: a local build cannot
    /// emit **even holding real credentials**. Analytics dry-runs and no crash
    /// client is constructed, so there is nothing for a stray `.env` to switch on.
    ///
    /// `#[serial]` because this mutates the process environment, which races any
    /// concurrent reader — the same reason [`crate::env`]'s tests are serial.
    #[test]
    #[serial]
    #[allow(
        clippy::used_underscore_binding,
        reason = "`_sentry` is underscore-prefixed because it is held only for its Drop; \
                  whether it was constructed at all is exactly what this test asserts"
    )]
    fn a_local_build_cannot_emit_even_holding_credentials() {
        unsafe {
            std::env::set_var("MIXPANEL_PROJECT_TOKEN", "a-token-that-must-not-be-used");
            std::env::set_var("SENTRY_DSN", "https://public@example.invalid/1");
        }

        let telemetry = Telemetry::new(&Version::new(0, 0, 0), Sinks::resolve(), None);
        assert!(
            matches!(telemetry.analytics, Analytics::DryRun),
            "a local build must never construct a live analytics client"
        );
        assert!(
            telemetry._sentry.is_none(),
            "a local build must never construct a crash client, DSN or not"
        );

        unsafe {
            std::env::remove_var("MIXPANEL_PROJECT_TOKEN");
            std::env::remove_var("SENTRY_DSN");
        }
    }

    #[test]
    #[allow(
        clippy::used_underscore_binding,
        reason = "see `a_local_build_cannot_emit_even_holding_credentials`"
    )]
    fn the_test_double_builds_nothing() {
        let telemetry = Telemetry::default();
        assert!(matches!(telemetry.analytics, Analytics::Off));
        assert!(telemetry._sentry.is_none());
    }

    /// Only a release build reports crashes — the asymmetry the type documents.
    #[test]
    fn only_a_release_build_reports_crashes() {
        assert!(Sinks::Production.reports_crashes());
        assert!(!Sinks::Development.reports_crashes());
    }
}
