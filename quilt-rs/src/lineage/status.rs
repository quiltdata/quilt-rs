use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::manifest::ManifestRow;

/// Describes modified states of a file
#[derive(Debug)]
pub enum Change {
    Modified(ManifestRow), // modified to what
    Added(ManifestRow),    // added what
    Removed(ManifestRow),  // removed what
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
    /// Local-only package: either no remote configured, or remote set but never pushed
    Local,
    Error,
}

/// Status of the package and working directory of the package
#[derive(Debug, Default)]
pub struct InstalledPackageStatus {
    /// Current commit vs upstream state
    pub upstream_state: UpstreamState,
    /// File changes vs current commit (visible + junky files only)
    pub changes: ChangeSet,
    /// Files matched by .quiltignore — (path, matched_by pattern)
    pub ignored_files: Vec<(PathBuf, String)>,
    /// Files in changes that are also flagged as junk — (path, suggested pattern)
    pub junky_changes: Vec<(PathBuf, String)>,
}

impl InstalledPackageStatus {
    pub fn new(upstream_state: UpstreamState, changes: ChangeSet) -> Self {
        Self {
            upstream_state,
            changes,
            ignored_files: Vec::new(),
            junky_changes: Vec::new(),
        }
    }

    pub fn error() -> Self {
        Self {
            upstream_state: UpstreamState::Error,
            ..Default::default()
        }
    }

    pub fn local() -> Self {
        Self {
            upstream_state: UpstreamState::Local,
            ..Default::default()
        }
    }
}
