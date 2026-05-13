use serde::Serialize;

use quilt_uri::Host;
use quilt_uri::Namespace;
use tauri::Emitter;

use crate::autopull::PausedReason;
use crate::telemetry::prelude::*;

/// Event names. Kept in lockstep with the UI's `listen(...)` calls.
pub const STATUS_EVENT: &str = "package-status-changed";
pub const LOGIN_REQUIRED_EVENT: &str = "autopull-login-required";

/// Payload emitted to the UI when a package's upstream state changes after
/// a watcher tick. Mirrors the camelCase shape of `RefreshedPackageStatus`
/// so the UI can reuse its existing per-package signal.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatusEvent {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
}

/// Backend → frontend / log surface for watcher results.
///
/// The trait keeps the watcher portable: production wires a Tauri emitter,
/// tests and a hypothetical headless daemon wire a logger.
pub trait StatusReporter: Send + Sync + 'static {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent);
    fn report_paused(&self, namespace: &Namespace, reason: PausedReason);
    fn report_login_required(&self, host: Option<&Host>);
}

/// Stderr/log-only reporter. Used in tests where no Tauri runtime is
/// available, and reserved for a future headless CLI daemon (see
/// `plans/autosync/01-autopull/approach.md` "Follow-ups").
#[cfg_attr(not(test), allow(dead_code))]
pub struct LogReporter;

impl StatusReporter for LogReporter {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        info!(
            "autopull: namespace={namespace} status={} has_changes={}",
            event.status, event.has_changes,
        );
    }

    fn report_paused(&self, namespace: &Namespace, reason: PausedReason) {
        info!("autopull: paused namespace={namespace} reason={reason:?}");
    }

    fn report_login_required(&self, host: Option<&Host>) {
        if let Some(h) = host {
            warn!("autopull: login required for {h}");
        } else {
            warn!("autopull: login required");
        }
    }
}

/// Production reporter: emits typed events on the Tauri event bus and
/// also logs so file-tail-style debugging still works.
pub struct TauriEventReporter {
    handle: tauri::AppHandle,
}

impl TauriEventReporter {
    pub fn new(handle: tauri::AppHandle) -> Self {
        Self { handle }
    }
}

impl StatusReporter for TauriEventReporter {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        info!(
            "autopull: namespace={namespace} status={} has_changes={}",
            event.status, event.has_changes,
        );
        if let Err(err) = self.handle.emit(STATUS_EVENT, &event) {
            warn!("autopull: failed to emit {STATUS_EVENT}: {err}");
        }
    }

    fn report_paused(&self, namespace: &Namespace, reason: PausedReason) {
        info!("autopull: paused namespace={namespace} reason={reason:?}");
        // A paused namespace is conveyed to the UI via the trailing status
        // emit in `tick.rs::run_once` — no dedicated event for now (the
        // approach doc relies on the existing `Behind`/`Diverged` render).
    }

    fn report_login_required(&self, host: Option<&Host>) {
        if let Some(h) = host {
            warn!("autopull: login required for {h}");
        } else {
            warn!("autopull: login required");
        }
        let payload = LoginRequiredEvent {
            host: host.map(ToString::to_string),
        };
        if let Err(err) = self.handle.emit(LOGIN_REQUIRED_EVENT, &payload) {
            warn!("autopull: failed to emit {LOGIN_REQUIRED_EVENT}: {err}");
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct LoginRequiredEvent {
    host: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_status_event_serializes_camel_case() {
        let event = PackageStatusEvent {
            namespace: "acme/demo".to_string(),
            status: "up_to_date".to_string(),
            has_changes: false,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains(r#""hasChanges":false"#),
            "expected camelCase `hasChanges`, got: {json}"
        );
        assert!(json.contains(r#""status":"up_to_date""#));
        assert!(json.contains(r#""namespace":"acme/demo""#));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct RecordingReporter {
        pub statuses: Mutex<Vec<(Namespace, PackageStatusEvent)>>,
        pub paused: Mutex<Vec<(Namespace, PausedReason)>>,
        pub logins: Mutex<Vec<Option<Host>>>,
    }

    impl StatusReporter for RecordingReporter {
        fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
            self.statuses
                .lock()
                .unwrap()
                .push((namespace.clone(), event));
        }

        fn report_paused(&self, namespace: &Namespace, reason: PausedReason) {
            self.paused
                .lock()
                .unwrap()
                .push((namespace.clone(), reason));
        }

        fn report_login_required(&self, host: Option<&Host>) {
            self.logins.lock().unwrap().push(host.cloned());
        }
    }
}
