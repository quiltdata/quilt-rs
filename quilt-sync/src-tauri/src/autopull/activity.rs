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
