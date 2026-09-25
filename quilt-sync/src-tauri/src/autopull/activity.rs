//! The transfer the autopull tick is running right now, for the window's
//! activity line.
//!
//! It is state, not deltas: one value that is either the pull or publish in
//! flight or nothing, and a reader that missed a change only needs the latest.
//! It is not the tray mode either — the icon's `Syncing` spans the whole tick,
//! the read-only check included, while this names only the transfer.

use quilt_uri::Namespace;
use serde::Deserialize;
use serde::Serialize;
use tauri::Emitter;
use tauri::async_runtime;
use tokio::sync::watch;

use crate::telemetry::prelude::*;

/// Event the window listens for to follow the activity. Kept in lockstep with
/// the UI's `listen(...)` call.
pub const AUTOPULL_ACTIVITY_EVENT: &str = "autopull-activity";

/// Which package autopull is moving files for, and which way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopullActivity {
    pub op: ActivityOp,
    pub namespace: Namespace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityOp {
    /// Applying a pull: getting the latest revision into the working tree.
    Pull,
    /// Publishing the working tree's changes as a new revision.
    Publish,
}

/// Forward the activity to the window: the whole value once now, then again
/// on every change — a set or a clear. A window that mounts later hydrates
/// with the `get_autopull_activity` command first, then follows this event.
pub fn spawn_forwarder(
    app: tauri::AppHandle,
    mut rx: watch::Receiver<Option<AutopullActivity>>,
) -> async_runtime::JoinHandle<()> {
    async_runtime::spawn(async move {
        let emit = |value: Option<AutopullActivity>| {
            if let Err(err) = app.emit(AUTOPULL_ACTIVITY_EVENT, value) {
                warn!("activity: failed to emit {AUTOPULL_ACTIVITY_EVENT}: {err}");
            }
        };
        emit(rx.borrow_and_update().clone());
        while rx.changed().await.is_ok() {
            emit(rx.borrow_and_update().clone());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_form_is_op_and_namespace() {
        let pull = AutopullActivity {
            op: ActivityOp::Pull,
            namespace: ("a", "b").into(),
        };
        assert_eq!(
            serde_json::to_string(&pull).unwrap(),
            r#"{"op":"pull","namespace":"a/b"}"#
        );
        let publish = AutopullActivity {
            op: ActivityOp::Publish,
            namespace: ("a", "b").into(),
        };
        assert_eq!(
            serde_json::to_string(&publish).unwrap(),
            r#"{"op":"publish","namespace":"a/b"}"#
        );
    }

    #[test]
    fn nothing_running_is_null() {
        assert_eq!(
            serde_json::to_string(&None::<AutopullActivity>).unwrap(),
            "null"
        );
    }
}
