use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::lineage::Change;
use crate::lineage::InstalledPackageStatus;
use crate::manifest::Manifest;
use crate::object_hash::ObjectHash;

/// A *dry-run* verdict of what a [`pull`](super::pull) **would** do, computed
/// from the working-tree changeset and the `base ↔ latest` manifest diff.
///
/// See `model/ctx/sync/node.md#pull-outcome` in the spec corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullOutcome {
    /// Nothing to pull: `base` and `latest` are the *same revision* (identical
    /// manifests). A newer metadata-only revision (same rows, different header)
    /// is **not** `UpToDate` — it is a [`CleanUpdate`](Self::CleanUpdate) (or
    /// [`KeepsLocalChanges`](Self::KeepsLocalChanges)) so its hashes advance.
    UpToDate,
    /// No local changes; a straight surgical update of remote-changed paths.
    CleanUpdate,
    /// The surgical update **and** the kept non-conflicting local work. The
    /// split is for messaging only, not logic.
    KeepsLocalChanges {
        added: Vec<PathBuf>,
        modified: Vec<PathBuf>,
        removed: Vec<PathBuf>,
    },
    /// A path changed on both sides with a different result — either a tracked
    /// path modified/removed on both sides, or a path *added* on both sides
    /// (present in `latest`, absent from `base`) with different content. Any
    /// single conflict blocks the whole (atomic) pull.
    Blocked { conflicts: Vec<PathBuf> },
}

/// How the remote changed a path between `base` and `latest`.
// The `Modified` payload (`ObjectHash`) is intentionally inline: this enum's
// shape is fixed by the plan and matched by later tasks, and it only ever
// lives in a small, transient dry-run delta map — boxing would add indirection
// for no real gain.
#[allow(clippy::large_enum_variant)]
pub(crate) enum RemoteChange {
    /// Remote content hash at `latest`.
    Modified(ObjectHash),
    Removed,
    /// Present in `latest`, absent from `base` — the remote grew a path.
    /// Carries the content hash at `latest`, like `Modified`.
    Added(ObjectHash),
}

/// The whole remote `base → latest` row delta: modified, removed, **and
/// added**.
///
/// Latest-only paths are included unconditionally, and whether a pull acts on
/// them is not decided here — [`pull`](super::pull) filters the touch set by
/// the caller's sync scope. That split matters: the delta is also the conflict
/// input for [`classify_pull`], which needs to see a remote *add* regardless of
/// whether this copy intends to fetch it, or a both-added collision would go
/// unnoticed.
///
/// This diffs manifest **rows** only, so a metadata-only revision (same rows,
/// different header) yields an empty delta. It therefore drives the surgical
/// touch set and conflict detection, but it is **not** the `UpToDate` signal —
/// [`classify_pull`] decides that from whole-manifest identity instead.
pub(crate) fn remote_delta(base: &Manifest, latest: &Manifest) -> BTreeMap<PathBuf, RemoteChange> {
    let mut delta = BTreeMap::new();
    for base_row in &base.rows {
        match latest.get_record(&base_row.logical_key) {
            Some(latest_row) if latest_row.hash == base_row.hash => {}
            Some(latest_row) => {
                delta.insert(
                    base_row.logical_key.clone(),
                    RemoteChange::Modified(latest_row.hash.clone()),
                );
            }
            None => {
                delta.insert(base_row.logical_key.clone(), RemoteChange::Removed);
            }
        }
    }
    for latest_row in &latest.rows {
        if base.get_record(&latest_row.logical_key).is_none() {
            delta.insert(
                latest_row.logical_key.clone(),
                RemoteChange::Added(latest_row.hash.clone()),
            );
        }
    }
    delta
}

/// Do the local and remote sides of a both-changed path reach the *same*
/// result? Same content (or both removed) is not a conflict.
fn same_resulting_content(local: &Change, remote: &RemoteChange) -> bool {
    match (local, remote) {
        (Change::Modified(row), RemoteChange::Modified(hash))
        // Both sides added the same path. Only `Change::Added` can pair with
        // `RemoteChange::Added`: a local add means the path has no `base` row,
        // and a remote add means the same, while `Modified`/`Removed` on either
        // side require one.
        | (Change::Added(row), RemoteChange::Added(hash)) => &row.hash == hash,
        (Change::Removed(_), RemoteChange::Removed) => true,
        _ => false,
    }
}

/// Classify what a pull would do. Pure — no network, no I/O.
///
/// `UpToDate` is returned only when `base` and `latest` are the same revision
/// (identical manifests). A newer metadata-only revision (same rows, different
/// header) is a `CleanUpdate`/`KeepsLocalChanges` so the pull advances the
/// hashes even though the surgical touch set is empty.
#[must_use]
pub fn classify_pull(
    status: &InstalledPackageStatus,
    base: &Manifest,
    latest: &Manifest,
) -> PullOutcome {
    // Same revision — identical manifests — is the only genuine "nothing to
    // pull". A newer revision that changed *only* the manifest header
    // (message / user_meta) has an empty row delta but is still something to
    // pull: its hashes must advance. Keying `UpToDate` off the row delta alone
    // would strand such a revision permanently `Behind`.
    if base == latest {
        return PullOutcome::UpToDate;
    }
    let delta = remote_delta(base, latest);
    if status.changes.is_empty() {
        // Includes the metadata-only case (empty `delta`): the surgical touch
        // set is empty, but the hashes still advance to `latest`.
        return PullOutcome::CleanUpdate;
    }

    let mut conflicts = Vec::new();
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut removed = Vec::new();

    for (path, change) in &status.changes {
        if let Some(remote_change) = delta.get(path) {
            // Changed on both sides: conflict unless the results agree.
            if !same_resulting_content(change, remote_change) {
                conflicts.push(path.clone());
            }
            // Same result → trivially resolved: not kept work, not a conflict.
            continue;
        }
        // No `remote_delta` entry means the remote left this path alone, for
        // every kind of local change — including an add, now that the delta
        // carries latest-only paths. (While it was base-only, an add had to
        // consult `latest` here to tell a local-only add from a hidden
        // both-added collision; the delta answers that itself.) So the local
        // change is carried forward.
        match change {
            Change::Added(_) => added.push(path.clone()),
            Change::Modified(_) => modified.push(path.clone()),
            Change::Removed(_) => removed.push(path.clone()),
        }
    }

    if conflicts.is_empty() {
        PullOutcome::KeepsLocalChanges {
            added,
            modified,
            removed,
        }
    } else {
        PullOutcome::Blocked { conflicts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use multihash::Multihash;

    use crate::Res;
    use crate::lineage::Change;
    use crate::lineage::ChangeSet;
    use crate::lineage::InstalledPackageStatus;
    use crate::lineage::UpstreamState;
    use crate::manifest::Manifest;
    use crate::manifest::ManifestRow;

    fn row(key: &str, hash_seed: &[u8]) -> ManifestRow {
        ManifestRow {
            logical_key: PathBuf::from(key),
            physical_key: format!("s3://b/{key}"),
            hash: Multihash::<256>::wrap(0x12, hash_seed)
                .unwrap()
                .try_into()
                .unwrap(),
            size: hash_seed.len() as u64,
            meta: None,
        }
    }

    fn manifest_of(rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            rows,
            ..Manifest::default()
        }
    }

    fn behind(changes: ChangeSet) -> InstalledPackageStatus {
        InstalledPackageStatus::new(UpstreamState::Behind, changes)
    }

    #[test(tokio::test)]
    async fn clean_tree_is_clean_update() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"2")]); // remote changed "a"
        let out = classify_pull(&behind(ChangeSet::default()), &base, &latest);
        assert_eq!(out, PullOutcome::CleanUpdate);
        Ok(())
    }

    #[test(tokio::test)]
    async fn added_file_is_kept() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"2")]);
        let mut changes = ChangeSet::new();
        changes.insert(
            PathBuf::from("new.txt"),
            Change::Added(row("new.txt", b"x")),
        );
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::KeepsLocalChanges {
                added: vec![PathBuf::from("new.txt")],
                modified: vec![],
                removed: vec![],
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn same_path_different_content_blocks() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"remote")]); // remote modified "a"
        let mut changes = ChangeSet::new();
        changes.insert(PathBuf::from("a"), Change::Modified(row("a", b"local"))); // local modified "a"
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::Blocked {
                conflicts: vec![PathBuf::from("a")]
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn identical_edit_is_not_a_conflict() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"same")]);
        let mut changes = ChangeSet::new();
        changes.insert(PathBuf::from("a"), Change::Modified(row("a", b"same"))); // same content
        let out = classify_pull(&behind(changes), &base, &latest);
        // Trivially resolved: neither conflict nor kept work.
        assert_eq!(
            out,
            PullOutcome::KeepsLocalChanges {
                added: vec![],
                modified: vec![],
                removed: vec![]
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn local_remove_vs_remote_modify_blocks() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"2")]); // remote modified "a"
        let mut changes = ChangeSet::new();
        changes.insert(PathBuf::from("a"), Change::Removed(row("a", b"1"))); // local removed "a"
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::Blocked {
                conflicts: vec![PathBuf::from("a")]
            }
        );
        Ok(())
    }

    // Mirror direction of the presence conflict above: the remote removed a
    // path the user modified locally. `same_resulting_content` has no explicit
    // arm for (Modified, Removed) — the catch-all keeps it a conflict; this
    // test pins that so an explicit-arms refactor can't silently drop the pair.
    #[test(tokio::test)]
    async fn local_modify_vs_remote_remove_blocks() -> Res {
        let base = manifest_of(vec![row("a", b"1"), row("b", b"2")]);
        let latest = manifest_of(vec![row("b", b"2")]); // remote removed "a"
        let mut changes = ChangeSet::new();
        changes.insert(PathBuf::from("a"), Change::Modified(row("a", b"local"))); // local modified "a"
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::Blocked {
                conflicts: vec![PathBuf::from("a")]
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn both_removed_is_not_a_conflict() -> Res {
        let base = manifest_of(vec![row("a", b"1"), row("b", b"2")]);
        let latest = manifest_of(vec![row("b", b"2")]); // remote removed "a"
        let mut changes = ChangeSet::new();
        changes.insert(PathBuf::from("a"), Change::Removed(row("a", b"1"))); // local removed "a"
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::KeepsLocalChanges {
                added: vec![],
                modified: vec![],
                removed: vec![]
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn both_added_different_content_blocks() -> Res {
        // "new.txt" is absent from `base` but added on BOTH sides with
        // different content. It has no `remote_delta` entry (delta is
        // base-only), so it must be caught via `latest.get_record` and treated
        // as a conflict, not silently kept.
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"1"), row("new.txt", b"remote")]);
        let mut changes = ChangeSet::new();
        changes.insert(
            PathBuf::from("new.txt"),
            Change::Added(row("new.txt", b"local")),
        );
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::Blocked {
                conflicts: vec![PathBuf::from("new.txt")]
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn both_added_same_content_is_trivially_resolved() -> Res {
        // "new.txt" added on both sides with identical content: like
        // `same_resulting_content`, it appears in neither the kept lists nor
        // the conflicts.
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"1"), row("new.txt", b"same")]);
        let mut changes = ChangeSet::new();
        changes.insert(
            PathBuf::from("new.txt"),
            Change::Added(row("new.txt", b"same")),
        );
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::KeepsLocalChanges {
                added: vec![],
                modified: vec![],
                removed: vec![],
            }
        );
        Ok(())
    }

    /// The delta reports all three kinds, additions included. It is
    /// deliberately scope-blind: a remote add must be visible here even for a
    /// caller that will not fetch it, because this is also the conflict input —
    /// [`classify_pull`] reads it to catch a both-added collision, which it
    /// would miss if additions were filtered out at the source.
    #[test]
    fn the_delta_reports_additions_alongside_modifications_and_removals() {
        let base = manifest_of(vec![
            row("kept", b"k"),
            row("gone", b"g"),
            row("edit", b"e1"),
        ]);
        let latest = manifest_of(vec![
            row("kept", b"k"),
            row("edit", b"e2"),
            row("new", b"n"),
        ]);

        let delta = remote_delta(&base, &latest);

        assert!(
            !delta.contains_key(&PathBuf::from("kept")),
            "an unchanged row is not a change"
        );
        assert!(matches!(
            delta.get(&PathBuf::from("gone")),
            Some(RemoteChange::Removed)
        ));
        assert!(matches!(
            delta.get(&PathBuf::from("edit")),
            Some(RemoteChange::Modified(_))
        ));
        assert!(
            matches!(delta.get(&PathBuf::from("new")), Some(RemoteChange::Added(h)) if *h == row("new", b"n").hash),
            "a latest-only path is an Added carrying its content hash"
        );
    }

    #[test(tokio::test)]
    async fn no_remote_change_is_up_to_date() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"1")]); // identical
        let out = classify_pull(&behind(ChangeSet::default()), &base, &latest);
        assert_eq!(out, PullOutcome::UpToDate);
        Ok(())
    }

    #[test(tokio::test)]
    async fn metadata_only_change_is_clean_update() -> Res {
        // Same file rows, newer manifest header (message differs). This is a
        // real revision to pull — hashes must advance — so it is a
        // `CleanUpdate`, never `UpToDate`, even though the row delta is empty.
        let base = manifest_of(vec![row("a", b"1")]);
        let mut latest = manifest_of(vec![row("a", b"1")]);
        latest.header.message = Some("newer revision message".to_string());
        let out = classify_pull(&behind(ChangeSet::default()), &base, &latest);
        assert_eq!(out, PullOutcome::CleanUpdate);
        Ok(())
    }

    #[test(tokio::test)]
    async fn metadata_only_change_keeps_local_changes() -> Res {
        // Metadata-only remote revision with an untouched-by-remote local add:
        // the local work is kept and the (empty) surgical update still proceeds.
        let base = manifest_of(vec![row("a", b"1")]);
        let mut latest = manifest_of(vec![row("a", b"1")]);
        latest.header.message = Some("newer revision message".to_string());
        let mut changes = ChangeSet::new();
        changes.insert(
            PathBuf::from("new.txt"),
            Change::Added(row("new.txt", b"x")),
        );
        let out = classify_pull(&behind(changes), &base, &latest);
        assert_eq!(
            out,
            PullOutcome::KeepsLocalChanges {
                added: vec![PathBuf::from("new.txt")],
                modified: vec![],
                removed: vec![],
            }
        );
        Ok(())
    }
}
