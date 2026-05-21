use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
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

impl fmt::Display for UpstreamState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            UpstreamState::Ahead => "ahead",
            UpstreamState::Behind => "behind",
            UpstreamState::Diverged => "diverged",
            UpstreamState::UpToDate => "up_to_date",
            UpstreamState::Local => "local",
            UpstreamState::Error => "error",
        };
        f.write_str(s)
    }
}

/// Status of the package and working directory of the package
#[derive(Debug, Default)]
pub struct InstalledPackageStatus {
    /// Current commit vs upstream state
    pub upstream_state: UpstreamState,
    /// File changes vs current commit (visible + junky files only)
    pub changes: ChangeSet,
    /// Files matched by `.quiltignore` — (path, `matched_by` pattern, size in bytes)
    pub ignored_files: Vec<(PathBuf, String, u64)>,
    /// Files in changes that are also flagged as junk — (path, suggested pattern)
    pub junky_changes: Vec<(PathBuf, String)>,
    /// Most recent `mtime` across non-ignored files in the working tree.
    /// `None` when the package has no such files. Used by autosync's
    /// quiet-window guard to skip the publish branch during a save burst.
    /// Future-dated `mtime`s are clamped to `now` at compare time in
    /// `working_tree_quiet`, not here.
    pub most_recent_mtime: Option<SystemTime>,
}

impl InstalledPackageStatus {
    pub fn new(upstream_state: UpstreamState, changes: ChangeSet) -> Self {
        Self {
            upstream_state,
            changes,
            ignored_files: Vec::new(),
            junky_changes: Vec::new(),
            most_recent_mtime: None,
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

    /// True when no tracked file was modified within `quiet_window` of
    /// `now`. Defensive: a `mtime` strictly later than `now` (clock
    /// skew, file dated in the future) is clamped to `now`.
    pub fn working_tree_quiet(&self, now: SystemTime, quiet_window: Duration) -> bool {
        let Some(mtime) = self.most_recent_mtime else {
            return true;
        };
        let mtime = mtime.min(now);
        now.duration_since(mtime).is_ok_and(|d| d >= quiet_window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_tree_quiet_no_files_is_true() {
        let status = InstalledPackageStatus::default();
        assert!(status.working_tree_quiet(SystemTime::now(), Duration::from_secs(30)));
    }

    #[test]
    fn working_tree_quiet_recent_mtime_is_not_quiet() {
        let now = SystemTime::now();
        let status = InstalledPackageStatus {
            most_recent_mtime: Some(now - Duration::from_secs(1)),
            ..InstalledPackageStatus::default()
        };
        assert!(!status.working_tree_quiet(now, Duration::from_secs(30)));
    }

    #[test]
    fn working_tree_quiet_old_mtime_is_quiet() {
        let now = SystemTime::now();
        let status = InstalledPackageStatus {
            most_recent_mtime: Some(now - Duration::from_secs(60)),
            ..InstalledPackageStatus::default()
        };
        assert!(status.working_tree_quiet(now, Duration::from_secs(30)));
    }

    #[test]
    fn working_tree_quiet_future_mtime_clamps_to_now() {
        let now = SystemTime::now();
        let status = InstalledPackageStatus {
            most_recent_mtime: Some(now + Duration::from_secs(300)),
            ..InstalledPackageStatus::default()
        };
        assert!(!status.working_tree_quiet(now, Duration::from_secs(30)));
    }
}
