use semver::Version;

use crate::env;
use crate::telemetry::{AmbientHost, InstallId, Sinks};

/// Where a fault goes — a signal that something should not have happened, which
/// no event describes.
///
/// A field rather than a direct call to the SDK, and that is the whole point: the
/// crash client is a process global, so reporting through it left **nothing a test
/// could observe**. Instrumenting the background engine turns mostly on failures,
/// and failure counting rides this sink rather than the analytics vocabulary, so
/// the mechanism the telemetry work leans on for failure visibility was the one
/// nothing could assert against.
///
/// Deliberately not a trait: this is a seam for *observation*, not an abstraction
/// over vendors — the sinks are three different shapes and the crash SDK actively
/// resists wrapping.
#[derive(Clone)]
pub enum Faults {
    /// A release build: captured by the crash client, if one was configured.
    Live,
    /// A local build: written to the developer's console, because there is no
    /// crash client to hold it. Today these vanish entirely in a dev build.
    DryRun,
    /// Tests: recorded in memory, so a test can assert that a path reported.
    #[cfg(test)]
    Recorded(std::sync::Arc<std::sync::Mutex<Vec<String>>>),
}

impl Faults {
    pub fn resolve(sinks: Sinks) -> Self {
        if sinks.reports_crashes() {
            Self::Live
        } else {
            Self::DryRun
        }
    }

    /// Report an anomaly: something that should not have happened, with no `Err`
    /// to carry it.
    ///
    /// `message` must stay constant — the crash reporter groups by it, so a
    /// variable part belongs in the host tag rather than the text, or one anomaly
    /// becomes one issue per host.
    pub fn anomaly(&self, message: &str) {
        match self {
            // The returned event id is discarded: nothing here correlates a fault
            // back to its report, and returning it would invite a caller to think
            // something does.
            Self::Live => {
                sentry::capture_message(message, sentry::Level::Warning);
            }
            Self::DryRun => eprintln!("telemetry(dry-run) anomaly: {message}"),
            #[cfg(test)]
            Self::Recorded(recorded) => Self::record(recorded, format!("anomaly: {message}")),
        }
    }

    /// Report a fault the caller is not failing on.
    pub fn error(&self, err: &(dyn std::error::Error + Send + Sync + 'static)) {
        match self {
            Self::Live => {
                sentry::capture_error(err);
            }
            Self::DryRun => eprintln!("telemetry(dry-run) fault: {err}"),
            #[cfg(test)]
            Self::Recorded(recorded) => Self::record(recorded, format!("fault: {err}")),
        }
    }

    #[cfg(test)]
    fn record(recorded: &std::sync::Mutex<Vec<String>>, entry: String) {
        // A poisoned lock would mean another test thread panicked mid-record.
        // Dropping the entry is right: the panic is the failure worth reporting,
        // and a second panic here would bury it.
        if let Ok(mut recorded) = recorded.lock() {
            recorded.push(entry);
        }
    }

    #[cfg(test)]
    pub fn reported(&self) -> Vec<String> {
        match self {
            Self::Recorded(recorded) => recorded.lock().map(|r| r.clone()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }
}

fn get_sentry_dsn() -> Option<sentry::types::Dsn> {
    env::sentry_dsn().and_then(|dsn_str| {
        dsn_str.parse().ok().or_else(|| {
            eprintln!("Warning: Invalid SENTRY_DSN format: {dsn_str}");
            None
        })
    })
}

/// Stamp every outgoing event with the deployment in play (read from `host`) and
/// this install's identity.
///
/// The hook is the only mechanism that holds for a crash: it runs on the single
/// process-wide client as the event leaves, so it covers a panic, a captured
/// error and a user-filed crash report alike, on any thread. Setting the scope
/// instead would reach only the thread that set it — see [`AmbientHost`].
///
/// The identity rides here for the same reason rather than because it varies: it
/// is fixed for the process, but a scope carrying it would still reach only the
/// threads that snapshotted after it was set. One mechanism, both facts.
fn stamp_event(install_id: Option<InstallId>, host: AmbientHost) -> sentry::ClientOptions {
    // Built once, not per event: the identity is fixed for the process, so the
    // hook clones a finished value rather than reassembling it on every send.
    // `user.id` and nothing else — the install identity is all we know, and it is
    // deliberately not a person.
    let user = install_id.map(|install_id| sentry::User {
        id: Some(install_id.as_str().to_owned()),
        ..Default::default()
    });

    let before_send = move |mut event: sentry::protocol::Event<'static>| {
        if let Some(host) = host.lock().ok().and_then(|host| host.clone()) {
            event
                .tags
                .insert("quilt_host".to_string(), host.to_string());
        }
        event.user.clone_from(&user);
        Some(event)
    };

    sentry::ClientOptions::new().before_send(before_send)
}

pub fn sentry_config(
    version: &Version,
    install_id: Option<InstallId>,
    host: AmbientHost,
) -> Option<sentry::ClientOptions> {
    let dsn = get_sentry_dsn();
    if dsn.is_none() {
        eprintln!("No SENTRY_DSN configured, Sentry disabled");
    }
    dsn.map(|dsn| {
        // `ClientOptions` is `#[non_exhaustive]` as of sentry 0.49, so it is built
        // through the setters. `dsn` is assigned directly because the `dsn` setter
        // takes a `&str` and panics on a malformed value — `get_sentry_dsn` already
        // parsed it and warns instead.
        //
        // A constant `environment`, because only a release build ever gets here —
        // see [`Sinks`](crate::telemetry::Sinks). Separating *kinds* of release
        // (an internal build from a customer's) is a distinct question and wants
        // more than two values, so it belongs to whoever takes that on.
        let mut options = stamp_event(install_id, host)
            .release(version.to_string())
            .environment("production");
        options.dsn = Some(dsn);
        options
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::Sinks;

    fn recorder() -> Faults {
        Faults::Recorded(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
    }

    /// Both kinds of fault are recorded, distinguishably and in order — a test
    /// asserting "an anomaly was reported" must not be satisfied by an error.
    #[test]
    fn records_anomalies_and_errors_apart() {
        let faults = recorder();

        faults.anomaly("a constant message");
        faults.error(&std::io::Error::other("something broke"));

        assert_eq!(
            faults.reported(),
            vec![
                "anomaly: a constant message".to_owned(),
                "fault: something broke".to_owned()
            ]
        );
    }

    /// A build that reports nowhere observable reports *nothing* to a reader —
    /// so a test holding a live or dry-run sink cannot accidentally pass by
    /// reading someone else's recording.
    #[test]
    fn only_the_recorder_reports() {
        assert!(Faults::Live.reported().is_empty());
        assert!(Faults::DryRun.reported().is_empty());
    }

    /// A local build has no crash client, so its faults go to the console rather
    /// than nowhere — which is what they did before.
    #[test]
    fn a_local_build_dry_runs_its_faults() {
        assert!(matches!(
            Faults::resolve(Sinks::Development),
            Faults::DryRun
        ));
        assert!(matches!(Faults::resolve(Sinks::Production), Faults::Live));
    }
}
