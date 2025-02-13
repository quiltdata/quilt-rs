use std::collections::BTreeMap;
use std::path::PathBuf;

use multihash::Multihash;
use serde::Serialize;

use crate::manifest::Row;

/// Describes modified states of a file
#[derive(Debug, PartialEq)]
pub enum Change {
    // TODO: Use Row
    Modified(PackageFileFingerprint), // modified to what
    // TODO: Use Row
    Added(PackageFileFingerprint), // added what
    Removed(Row),                  // removed what
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

/// Some auxiliary struct that we use instead of `Row` when the file is not yet commited
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackageFileFingerprint {
    // FIXME: re-use Row
    pub size: u64,
    pub hash: Multihash<256>,
}

/// Status of the package and workding directory of the pakage
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
