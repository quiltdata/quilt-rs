use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::Res;
use crate::checksum::refresh_hash;
use crate::io::storage::Storage;
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

/// The dry run's verdict, plus **what the incoming revision adds** — the paths
/// present in `latest` and absent from `base`.
///
/// A wrapper rather than a field on [`PullOutcome`], whose variants answer
/// "would this pull be safe" and cross the wire to the desktop; what a revision
/// brings is orthogonal to that verdict and true whatever it says.
///
/// Scope-independent on purpose: this reports what the revision *holds*, and
/// whether a pull would fetch it is [`SyncScope`](crate::lineage::SyncScope)'s
/// business at apply time. It costs nothing — the manifest naming these paths
/// was already fetched to reach the verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullPreview {
    pub outcome: PullOutcome,
    pub added: Vec<PathBuf>,
}

/// The paths `latest` holds that `base` does not.
#[must_use]
pub(crate) fn remote_additions(base: &Manifest, latest: &Manifest) -> Vec<PathBuf> {
    latest
        .rows
        .iter()
        .filter(|row| base.get_record(&row.logical_key).is_none())
        .map(|row| row.logical_key.clone())
        .collect()
}

/// Do the local and remote sides of a both-changed path reach the *same*
/// result? Same content (or both removed) is not a conflict.
fn same_resulting_content(local: &Change, remote: &RemoteChange, identical: bool) -> bool {
    match (local, remote) {
        (Change::Modified(row), RemoteChange::Modified(hash))
        // Both sides added the same path. Only `Change::Added` can pair with
        // `RemoteChange::Added`: a local add means the path has no `base` row,
        // and a remote add means the same, while `Modified`/`Removed` on either
        // side require one.
        // `identical` first: a digest comparison is only meaningful when both
        // sides are in the same algorithm, and `identical_to_latest` has
        // already settled the paths where they are not.
        | (Change::Added(row), RemoteChange::Added(hash)) => identical || &row.hash == hash,
        (Change::Removed(_), RemoteChange::Removed) => true,
        _ => false,
    }
}

/// The locally changed paths whose working file already holds *exactly* what
/// `latest` holds, for the paths where comparing the two digests cannot answer
/// that.
///
/// An [`ObjectHash`] is a multihash: it carries the algorithm that produced it,
/// and two algorithms' digests of identical bytes are unequal. A package's rows
/// can be in mixed algorithms — a changed file is hashed into the *host's*
/// declared algorithm at status time, an untouched row keeps whatever wrote it,
/// and `recommit` rehashes in bulk — so a local change and `latest`'s row for
/// the same path routinely disagree on algorithm. Left to a plain `==`, a file
/// byte-identical to `latest` reads as different content and blocks the pull.
///
/// This re-derives the answer the only way it can be derived: hash the working
/// file **in `latest`'s row's own algorithm**, which is what `refresh_hash`
/// does with a row — the same discipline the verify-before-uninstall pass in
/// [`pull`](super::pull) uses against the base row. `Ok(None)` from it means
/// the file already matches that row.
///
/// I/O only where the digests cannot be compared: a path whose algorithms
/// already agree is left to [`classify_pull`]'s own comparison, so a package
/// and host in step hash nothing here. Kept out of [`classify_pull`] so that
/// stays pure and synchronous.
///
/// A working file that cannot be read is simply not reported identical: the
/// path falls through to the digest comparison and, at worst, blocks the pull
/// as it does today. Fail-safe in the same direction as the conflict rule.
pub(crate) async fn identical_to_latest(
    storage: &(impl Storage + Sync),
    working_dir: &Path,
    status: &InstalledPackageStatus,
    base: &Manifest,
    latest: &Manifest,
) -> Res<BTreeSet<PathBuf>> {
    let mut identical = BTreeSet::new();
    for (path, change) in &status.changes {
        let local_hash = match change {
            Change::Modified(row) | Change::Added(row) => &row.hash,
            // A local removal is compared against a remote removal, which needs
            // no content at all.
            Change::Removed(_) => continue,
        };
        let Some(latest_row) = latest.get_record(path) else {
            continue;
        };
        // Only a path the remote also changed reaches the comparison at all —
        // [`classify_pull`] carries every other local change forward untouched.
        // Without this, a package whose rows are all in the other algorithm
        // would re-hash every locally edited file on every pull for an answer
        // nothing reads.
        if base
            .get_record(path)
            .is_some_and(|base_row| base_row.hash == latest_row.hash)
        {
            continue;
        }
        if local_hash.algorithm() == latest_row.hash.algorithm() {
            continue;
        }
        match refresh_hash(storage, &working_dir.join(path), latest_row.clone()).await {
            Ok(None) => {
                identical.insert(path.clone());
            }
            Ok(Some(_)) => {}
            Err(err) if err.is_not_found() => {}
            Err(err) => return Err(err),
        }
    }
    Ok(identical)
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
    identical: &BTreeSet<PathBuf>,
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
            if !same_resulting_content(change, remote_change, identical.contains(path)) {
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

    use aws_sdk_s3::primitives::ByteStream;
    use multihash::Multihash;

    use crate::checksum::calculate_hash;
    use crate::io::remote::HostChecksums;
    use crate::io::remote::HostConfig;
    use crate::io::storage::mocks::MockStorage;

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

    /// Only the latest-only paths, which is what lets a surface name what an
    /// unpulled revision brings. A path both sides hold is not news, and a path
    /// only `base` holds was removed rather than added.
    #[test]
    fn remote_additions_names_only_what_latest_gained() {
        let base = manifest_of(vec![row("kept.csv", b"1"), row("dropped.csv", b"2")]);
        let latest = manifest_of(vec![
            row("kept.csv", b"1"),
            row("added.csv", b"3"),
            row("also-added.csv", b"4"),
        ]);
        assert_eq!(
            remote_additions(&base, &latest),
            vec![PathBuf::from("added.csv"), PathBuf::from("also-added.csv")]
        );
    }

    /// A path whose *content* changed is a modification, not an addition — the
    /// comparison is on presence, so a differing hash must not leak in.
    #[test]
    fn a_changed_path_is_not_an_addition() {
        let base = manifest_of(vec![row("same-name.csv", b"before")]);
        let latest = manifest_of(vec![row("same-name.csv", b"after")]);
        assert!(remote_additions(&base, &latest).is_empty());
    }

    fn behind(changes: ChangeSet) -> InstalledPackageStatus {
        InstalledPackageStatus::new(UpstreamState::Behind, changes)
    }

    /// The incident's arm. The package's rows were written under one checksum
    /// algorithm; this host declares the other, so a locally edited file is
    /// hashed into the host's algorithm at status time. When that edit lands
    /// *exactly* the content `latest` holds, the two digests describe the same
    /// bytes and still compare unequal, because an `ObjectHash` carries its
    /// algorithm. Today that reads as a conflict and blocks the pull.
    #[test(tokio::test)]
    async fn identical_edit_across_algorithms_is_not_a_conflict() -> Res {
        let storage = MockStorage::default();
        let working_dir = PathBuf::from("/wd");
        let path = PathBuf::from("a");

        // The working file holds precisely what `latest` holds.
        let content = b"exactly the remote's new bytes";
        storage
            .write_byte_stream(working_dir.join(&path), ByteStream::from_static(content))
            .await?;

        // `latest`'s row for `a` is in CRC64 — whoever pushed it ran against a
        // host declaring that algorithm.
        let latest_row = calculate_hash(
            &storage,
            &working_dir.join(&path),
            &path,
            &HostConfig {
                checksums: HostChecksums::Crc64,
                host: None,
            },
        )
        .await?;
        // This host declares SHA-256-chunked, so `verify_hash` produced the
        // local change's row in that algorithm — same bytes, different digest.
        let local_row = calculate_hash(
            &storage,
            &working_dir.join(&path),
            &path,
            &HostConfig {
                checksums: HostChecksums::Sha256Chunked,
                host: None,
            },
        )
        .await?;
        assert_ne!(
            local_row.hash, latest_row.hash,
            "precondition: the two algorithms must disagree on the same bytes"
        );

        let base = manifest_of(vec![row("a", b"the old content")]);
        let latest = manifest_of(vec![latest_row]);
        let status = behind(ChangeSet::from([(
            path.clone(),
            Change::Modified(local_row),
        )]));

        let identical =
            identical_to_latest(&storage, &working_dir, &status, &base, &latest).await?;
        let out = classify_pull(&status, &base, &latest, &identical);

        assert_eq!(
            out,
            PullOutcome::KeepsLocalChanges {
                added: vec![],
                modified: vec![],
                removed: vec![],
            },
            "an edit landing latest's own content is trivially resolved, not a conflict"
        );
        Ok(())
    }

    /// The regression guard for the arm above: when the local edit is genuinely
    /// different content, a cross-algorithm package must still block. The fix
    /// may not turn every mismatched-algorithm path into a free pass.
    #[test(tokio::test)]
    async fn differing_edit_across_algorithms_still_blocks() -> Res {
        let storage = MockStorage::default();
        let working_dir = PathBuf::from("/wd");
        let path = PathBuf::from("a");

        // The working file holds the user's own edit.
        storage
            .write_byte_stream(
                working_dir.join(&path),
                ByteStream::from_static(b"the user's own edit"),
            )
            .await?;
        let local_row = calculate_hash(
            &storage,
            &working_dir.join(&path),
            &path,
            &HostConfig {
                checksums: HostChecksums::Sha256Chunked,
                host: None,
            },
        )
        .await?;

        // `latest` holds different content again, hashed under the other
        // algorithm.
        let other = PathBuf::from("other");
        storage
            .write_byte_stream(
                working_dir.join(&other),
                ByteStream::from_static(b"what the remote actually pushed"),
            )
            .await?;
        let mut latest_row = calculate_hash(
            &storage,
            &working_dir.join(&other),
            &path,
            &HostConfig {
                checksums: HostChecksums::Crc64,
                host: None,
            },
        )
        .await?;
        latest_row.logical_key = path.clone();

        let base = manifest_of(vec![row("a", b"the old content")]);
        let latest = manifest_of(vec![latest_row]);
        let status = behind(ChangeSet::from([(
            path.clone(),
            Change::Modified(local_row),
        )]));

        let identical =
            identical_to_latest(&storage, &working_dir, &status, &base, &latest).await?;
        let out = classify_pull(&status, &base, &latest, &identical);

        assert_eq!(
            out,
            PullOutcome::Blocked {
                conflicts: vec![path]
            },
            "a real both-changed disagreement must still block"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn clean_tree_is_clean_update() -> Res {
        let base = manifest_of(vec![row("a", b"1")]);
        let latest = manifest_of(vec![row("a", b"2")]); // remote changed "a"
        let out = classify_pull(
            &behind(ChangeSet::default()),
            &base,
            &latest,
            &BTreeSet::new(),
        );
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
        let out = classify_pull(
            &behind(ChangeSet::default()),
            &base,
            &latest,
            &BTreeSet::new(),
        );
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
        let out = classify_pull(
            &behind(ChangeSet::default()),
            &base,
            &latest,
            &BTreeSet::new(),
        );
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
        let out = classify_pull(&behind(changes), &base, &latest, &BTreeSet::new());
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
