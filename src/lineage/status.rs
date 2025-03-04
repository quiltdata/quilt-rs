use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::manifest::Row;

/// Describes modified states of a file
#[derive(Debug, PartialEq)]
pub enum Change {
    Modified(Row), // modified to what
    Added(Row),    // added what
    Removed(Row),  // removed what
}

/// Map of all changed files
pub type ChangeSet = BTreeMap<PathBuf, Change>;

/// State of the local package relative to the remote package
#[derive(Debug, PartialEq, Eq, Default, Serialize)]
pub enum UpstreamState {
    #[default]
    UpToDate,
    Behind,
    Ahead,
    Diverged,
}

/// Status of the package and working directory of the package
#[derive(Debug, PartialEq, Default)]
pub struct InstalledPackageStatus {
    /// Current commit vs upstream state
    pub upstream_state: UpstreamState,
    /// File changes vs current commit
    pub changes: ChangeSet,
    // XXX: meta?
}

impl InstalledPackageStatus {
    pub fn new(upstream_state: UpstreamState, changes: ChangeSet) -> Self {
        Self {
            upstream_state,
            changes,
        }
    }
}
