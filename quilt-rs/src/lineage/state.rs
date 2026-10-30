//! A package's state as a front end shows it: [`UpstreamState`] plus the facts
//! that split it, resolved in one place so every surface agrees.

use crate::lineage::PackageLineage;
use crate::lineage::UpstreamState;

/// A package's resolved state: what `QuiltSync`'s main page and
/// `quilt list --fetch` show for it.
///
/// Only the states a package's lineage and working tree can answer. States a
/// front end learns elsewhere — a paused sync, a pull conflict, a denied role,
/// a missing session — are that front end's to add on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageState {
    /// The installed revision is the remote's `latest`, and nothing local is
    /// waiting to be published.
    Latest,
    /// A local revision is not on the remote yet.
    PendingCommit,
    /// Files in the working tree changed since the last commit.
    PendingChanges { files: usize },
    /// The remote has a newer revision.
    Behind,
    /// The remote and the local copy both moved.
    Diverged,
    /// A remote is set, but nothing was published to it yet.
    Unpublished,
    /// No remote is set.
    NoRemote,
    /// The state could not be read: [`UpstreamState::Error`].
    Unknown,
}

impl PackageState {
    /// **Maps** the [`UpstreamState`] `quilt-rs` already derived.
    ///
    /// It does NOT re-derive from hashes. `UpstreamState` is computed from a
    /// lineage once (`UpstreamState::from(lineage)`, or by
    /// [`InstalledPackage::status`](crate::InstalledPackage::status)), and a
    /// second resolver deciding what is true is the bug this exists to prevent:
    /// resolution happens exactly once, upstream of every surface.
    ///
    /// The two booleans are not a second resolution: they split `UpstreamState`
    /// variants that this vocabulary distinguishes and `UpstreamState` does not.
    ///
    /// `changed_files` is what a status call measured — `None` when nobody
    /// looked at the working tree yet. It only ever outranks the two states that
    /// say nothing about the remote having moved (`Latest` and `PendingCommit`):
    /// `Diverged` and `Behind` outrank it, and a package with nowhere to publish
    /// to has no use for a file count.
    #[must_use]
    pub fn resolve(
        upstream: UpstreamState,
        has_local_commit: bool,
        has_remote: bool,
        changed_files: Option<usize>,
    ) -> Self {
        // A working tree measured as non-empty. See the doc comment for its rank.
        let pending_changes = match changed_files {
            Some(files) if files > 0 => Some(Self::PendingChanges { files }),
            _ => None,
        };

        match upstream {
            // `Local` means either no bucket chosen, or a bucket with nothing in it
            // yet. This vocabulary has a word for each.
            UpstreamState::Local if has_remote => Self::Unpublished,
            UpstreamState::Local => Self::NoRemote,
            UpstreamState::Behind => Self::Behind,
            UpstreamState::Diverged => Self::Diverged,
            UpstreamState::Error => Self::Unknown,
            UpstreamState::Ahead => pending_changes.unwrap_or(Self::PendingCommit),
            UpstreamState::UpToDate => pending_changes.unwrap_or({
                if has_local_commit {
                    Self::PendingCommit
                } else {
                    Self::Latest
                }
            }),
        }
    }
}

/// What [`InstalledPackage::state`](crate::InstalledPackage::state) learned
/// about a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageStateReport {
    pub state: PackageState,
    /// The [`UpstreamState`] `state` was resolved from: the status call's, or
    /// the lineage's when the package has no remote.
    pub upstream_state: UpstreamState,
    /// Files changed since the last commit. `None` when the working tree was
    /// not scanned: the package has no remote.
    pub changed_files: Option<usize>,
    /// The remote's `latest` tip was read in this call
    /// ([`InstalledPackageStatus::latest_refreshed`](crate::lineage::InstalledPackageStatus::latest_refreshed)).
    /// `false` when the remote was not asked, or could not be reached and the
    /// state was read against the last-known tip.
    pub latest_refreshed: bool,
}

impl PackageStateReport {
    /// The state of a package with no remote, resolved from its lineage alone.
    /// `None` when the remote has to be asked.
    #[must_use]
    pub(crate) fn without_remote(lineage: &PackageLineage) -> Option<Self> {
        // No remote at all: `resolve` ignores the file count for `Local`, so a
        // status call would be a working-tree walk whose answer nothing reads. A
        // remote that exists but was never pushed to is still asked — it is
        // reachable, and `Unpublished` is what comes back.
        if lineage.remote_uri.is_none() {
            let upstream_state = UpstreamState::from(lineage.clone());
            return Some(Self {
                state: PackageState::resolve(upstream_state, lineage.commit.is_some(), false, None),
                upstream_state,
                changed_files: None,
                latest_refreshed: false,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_uri::Host;
    use quilt_uri::ManifestUri;
    use quilt_uri::fixtures;

    fn resolve(
        upstream: UpstreamState,
        has_local_commit: bool,
        has_remote: bool,
        changed_files: Option<usize>,
    ) -> PackageState {
        PackageState::resolve(upstream, has_local_commit, has_remote, changed_files)
    }

    #[test]
    fn local_with_no_bucket_is_no_remote() {
        assert_eq!(
            resolve(UpstreamState::Local, false, false, None),
            PackageState::NoRemote
        );
    }

    #[test]
    fn local_with_a_bucket_is_unpublished() {
        // `UpstreamState::Local` covers BOTH "no remote configured" and "remote set
        // but never pushed" — its own doc comment says so. This splits them.
        assert_eq!(
            resolve(UpstreamState::Local, false, true, None),
            PackageState::Unpublished
        );
    }

    #[test]
    fn up_to_date_with_nothing_local_is_latest() {
        assert_eq!(
            resolve(UpstreamState::UpToDate, false, true, None),
            PackageState::Latest
        );
    }

    #[test]
    fn up_to_date_with_a_local_revision_is_pending_commit() {
        assert_eq!(
            resolve(UpstreamState::UpToDate, true, true, None),
            PackageState::PendingCommit
        );
    }

    #[test]
    fn ahead_is_pending_commit() {
        assert_eq!(
            resolve(UpstreamState::Ahead, false, true, None),
            PackageState::PendingCommit
        );
    }

    #[test]
    fn behind_carries_no_count() {
        assert_eq!(
            resolve(UpstreamState::Behind, false, true, None),
            PackageState::Behind,
            "a hash inequality is not a distance; no revision count is derivable"
        );
    }

    #[test]
    fn diverged_maps_straight_through() {
        assert_eq!(
            resolve(UpstreamState::Diverged, false, true, None),
            PackageState::Diverged
        );
    }

    #[test]
    fn error_becomes_the_fallback() {
        assert_eq!(
            resolve(UpstreamState::Error, false, true, None),
            PackageState::Unknown
        );
    }

    #[test]
    fn a_measured_working_tree_beats_an_unpushed_revision() {
        // Both offer to publish; only one of them can say how much.
        assert_eq!(
            resolve(UpstreamState::UpToDate, true, true, Some(3)),
            PackageState::PendingChanges { files: 3 }
        );
        assert_eq!(
            resolve(UpstreamState::Ahead, false, true, Some(2)),
            PackageState::PendingChanges { files: 2 }
        );
    }

    #[test]
    fn a_measured_clean_tree_falls_through_to_the_revision_state() {
        assert_eq!(
            resolve(UpstreamState::UpToDate, false, true, Some(0)),
            PackageState::Latest
        );
        assert_eq!(
            resolve(UpstreamState::UpToDate, true, true, Some(0)),
            PackageState::PendingCommit
        );
        assert_eq!(
            resolve(UpstreamState::Ahead, false, true, Some(0)),
            PackageState::PendingCommit
        );
    }

    #[test]
    fn a_count_never_outranks_a_state_above_it() {
        // `Diverged` and `Behind` outrank a file count. The local edits are real
        // and they are not what the state is about.
        assert_eq!(
            resolve(UpstreamState::Behind, false, true, Some(4)),
            PackageState::Behind
        );
        assert_eq!(
            resolve(UpstreamState::Diverged, false, true, Some(4)),
            PackageState::Diverged
        );
        // No bucket to publish to: the number is not the thing to say.
        assert_eq!(
            resolve(UpstreamState::Local, false, false, Some(4)),
            PackageState::NoRemote
        );
        assert_eq!(
            resolve(UpstreamState::Local, false, true, Some(4)),
            PackageState::Unpublished
        );
    }

    #[test]
    fn an_unmeasured_tree_agrees_with_a_measured_empty_one() {
        // Deliberate: `None` exists so a caller that has not looked cannot assert
        // a clean tree, not to produce a third state. If a future arm makes these
        // differ, that is a decision to take on purpose — this test is the tripwire.
        for upstream in [
            UpstreamState::UpToDate,
            UpstreamState::Ahead,
            UpstreamState::Behind,
            UpstreamState::Diverged,
            UpstreamState::Local,
            UpstreamState::Error,
        ] {
            for has_local_commit in [true, false] {
                assert_eq!(
                    resolve(upstream, has_local_commit, true, None),
                    resolve(upstream, has_local_commit, true, Some(0)),
                    "{upstream:?} / commit={has_local_commit} disagreed"
                );
            }
        }
    }

    fn manifest_uri(origin: Option<Host>) -> ManifestUri {
        ManifestUri {
            origin,
            bucket: "acme-research".to_string(),
            namespace: ("team", "one").into(),
            hash: "abcdef".to_string(),
        }
    }

    #[test]
    fn without_remote_answers_only_when_the_remote_is_not_asked() {
        assert_eq!(
            PackageStateReport::without_remote(&PackageLineage::default()),
            Some(PackageStateReport {
                state: PackageState::NoRemote,
                upstream_state: UpstreamState::Local,
                changed_files: None,
                latest_refreshed: false,
            })
        );

        // A remote with or without a catalog host is asked.
        let configured =
            PackageLineage::from_remote(manifest_uri(Some(fixtures::host())), "abcdef".to_string());
        assert_eq!(PackageStateReport::without_remote(&configured), None);
        let catalog_less = PackageLineage::from_remote(manifest_uri(None), "abcdef".to_string());
        assert_eq!(PackageStateReport::without_remote(&catalog_less), None);
    }
}
