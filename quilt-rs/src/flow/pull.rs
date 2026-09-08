use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use tracing::error;
use tracing::info;

use crate::Res;
use crate::checksum::refresh_hash;
use crate::error::PackageOpError;
use crate::flow;
use crate::flow::Applied;
use crate::flow::PullOutcome;
use crate::flow::apply_latest_update;
use crate::flow::classify_pull;
use crate::flow::pull_outcome::RemoteChange;
use crate::flow::remote_delta;
use crate::io::manifest::resolve_tag;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::ChangeSet;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineagePaths;
use crate::lineage::PackageLineage;
use crate::lineage::SyncScope;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::Tag;

/// A classification-ready snapshot for pull: the resolved `latest` and the
/// working-tree status, taken in that order — network first, walk last — so the
/// status is as fresh as possible when the classifier consumes it.
///
/// Always construct via [`snapshot_for_pull`], which performs every network
/// round-trip (tag resolve + manifest fetch) *before* the status walk. Building
/// one by hand outside tests defeats the freshness contract this type exists to
/// enforce.
#[derive(Debug)]
pub struct PullSnapshot {
    /// Working-tree status — the walk taken last, after the fetch.
    pub status: InstalledPackageStatus,
    /// The resolved `latest` (carries `.hash`).
    pub latest: ManifestUri,
    /// The `latest` manifest, parsed and already cached on disk.
    pub latest_manifest: Manifest,
}

/// Builds a [`PullSnapshot`] with all network done before the working-tree
/// walk, so the status is the freshest input the classifier sees.
///
/// Order matters: resolve `latest` (the one tag read that feeds both the
/// lineage and the fetch), then download + cache the manifest, then walk the
/// tree last. The tag resolution here **replaces** any separate
/// `refresh_latest_hash` call: one resolution updates `lineage.latest_hash` and
/// drives the fetch, closing the window where two independent reads could see a
/// tag move between them.
///
/// Returns the lineage with a refreshed `latest_hash` alongside the snapshot.
///
/// # Errors
/// Returns [`PackageOpError::AlreadyUpToDate`] when the resolved `latest`
/// already equals `base_hash` (no fetch or walk is paid for in that case).
/// Otherwise propagates tag-resolution, manifest-fetch, and status-walk errors.
pub async fn snapshot_for_pull(
    mut lineage: PackageLineage,
    base_manifest: &Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    package_home: impl AsRef<Path>,
    host_config: HostConfig,
) -> Res<(PackageLineage, PullSnapshot)> {
    // The ONE tag read. Its result both refreshes the lineage and names the
    // manifest to fetch — mirroring `refresh_latest_hash`'s requirement of a
    // remote (`remote()?` errors for a local-only package).
    let remote_uri = lineage.remote()?.clone();
    let origin = remote_uri.origin.clone();
    let latest = resolve_tag(remote, origin.as_ref(), remote_uri, Tag::Latest).await?;
    lineage.latest_hash.clone_from(&latest.hash);

    // Short-circuit before paying for the manifest fetch or the walk: if the
    // resolved `latest` is the base we already have, there is nothing to pull.
    if latest.hash == lineage.base_hash {
        return Err(PackageOpError::AlreadyUpToDate.into());
    }

    // Fetch + cache + parse the `latest` manifest.
    let latest_manifest = flow::cache_remote_manifest(paths, storage, remote, &latest).await?;

    // THE WALK, last — so `status` reflects the tree as of just before the
    // caller classifies and applies.
    let (lineage, status) =
        flow::status(lineage, storage, base_manifest, package_home, host_config).await?;

    Ok((
        lineage,
        PullSnapshot {
            status,
            latest,
            latest_manifest,
        },
    ))
}

/// What a pull applied, grouped by what happened to *this copy* rather than by
/// the shape of the remote's diff — under a sparse
/// [`SyncScope`] those differ, and the difference is the part worth reporting.
///
/// Three dispositions deliberately appear in no group, because nothing moved:
/// a path changed on both sides to the same result (trivially resolved), a
/// tracked path the user had edited (their work is kept, the remote's version
/// not applied), and a metadata-only revision (an empty touch set). A `Blocked`
/// pull produces no report at all — it applies nothing.
///
/// It describes a **span**, not a revision: a pull advances `base` straight to
/// `latest`, so one report can cover several revisions, and `message` is the
/// newest one's only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullReport {
    /// The revision the pull advanced to.
    pub manifest_uri: ManifestUri,
    /// Added by the remote and fetched — whole-package scope only, since an
    /// added path is never already tracked.
    pub added: Vec<PathBuf>,
    /// Added by the remote and left on it — individual-file scope only.
    pub added_not_fetched: Vec<PathBuf>,
    /// A tracked path the remote changed, rewritten from `latest`.
    pub updated: Vec<PathBuf>,
    /// A tracked path the remote dropped, deleted from the working tree.
    pub removed: Vec<PathBuf>,
    /// The newest revision's own message, empty treated as absent.
    pub message: Option<String>,
}

impl PullReport {
    /// Whether anything at all moved or is newly listed. False for a
    /// metadata-only revision, whose hashes advance and whose files do not.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.added_not_fetched.is_empty()
            && self.updated.is_empty()
            && self.removed.is_empty()
    }
}

/// Build the report from the delta and from what the apply actually did — no
/// extra I/O, both are in hand.
///
/// **Not from the touch set.** The touch set is what the pull *proposed* to
/// move, and the two diverge: a touched path this copy does not track is never
/// uninstalled, so reading the touch set reported files as removed that were
/// still there. [`Applied`](crate::flow::Applied) is the record of the writes
/// and deletions themselves, which makes every group true by construction.
fn report_of(
    manifest_uri: ManifestUri,
    delta: &BTreeMap<PathBuf, RemoteChange>,
    applied: &Applied,
    locally_changed: &ChangeSet,
    message: Option<String>,
) -> PullReport {
    let installed: BTreeSet<&PathBuf> = applied.installed.iter().collect();
    let uninstalled: BTreeSet<&PathBuf> = applied.uninstalled.iter().collect();
    let mut report = PullReport {
        manifest_uri,
        added: Vec::new(),
        added_not_fetched: Vec::new(),
        updated: Vec::new(),
        removed: Vec::new(),
        // An empty message is absent, which the desktop already assumes
        // elsewhere: `ManifestHeader::default` writes `Some(String::new())`.
        message: message.filter(|m| !m.is_empty()),
    };
    // Walked over the delta rather than over `applied`, so the report stays
    // ordered by path and covers only what the remote changed.
    for (path, change) in delta {
        match (installed.contains(path), uninstalled.contains(path)) {
            // Deleted and written again: a file that was here has new content.
            (true, true) => report.updated.push(path.clone()),
            // Written where there was nothing. The remote may have called this
            // path Added or Modified — under whole-package scope a path the
            // remote modified but this copy never checked out is fetched here
            // too, and it is new to this copy either way. Nothing was
            // overwritten, so "updated" would be the wrong word for it.
            (true, false) => report.added.push(path.clone()),
            // Deleted with no replacement: absent from `latest`.
            (false, true) => report.removed.push(path.clone()),
            (false, false) => {
                // Nothing moved. Worth saying only when the revision added a
                // path that is still outstanding — one the scope listed and
                // left on the remote.
                //
                // Everything else is silent, and each silence is a case where
                // the file on disk is already right: the user's own edit was
                // kept, both sides reached the same result (including a path
                // both added identically, which is here and needs no fetch),
                // or this copy does not track the path at all — the ordinary
                // state of a CLI install, since install brings the manifest
                // and not the files.
                if matches!(change, RemoteChange::Added(_)) && !locally_changed.contains_key(path) {
                    report.added_not_fetched.push(path.clone());
                }
            }
        }
    }
    report
}

/// Which of the remote's changed paths this pull will actually apply.
///
/// The **whole** of what a [`SyncScope`] does, in one place and free of I/O so
/// the rule is checkable without a working tree. Two filters, and they answer
/// different questions:
///
/// - the scope decides whether a path this copy does not track is in play at
///   all — sparse checkout under [`SyncScope::IndividualFiles`], everything the
///   remote touched under [`SyncScope::EntirePackage`], which is what lets a
///   remote *addition* be fetched;
/// - a path the user changed locally is dropped under **either** scope. The
///   classifier has already blocked the pull if that change disagrees with the
///   remote's, so what is left here is work to keep in place, not to overwrite.
///   Whole-package scope widens what a pull *fetches*; it never widens what a
///   pull is willing to clobber.
fn touch_set(
    remote_changed: impl IntoIterator<Item = PathBuf>,
    tracked: &LineagePaths,
    locally_changed: &ChangeSet,
    scope: SyncScope,
) -> Vec<PathBuf> {
    remote_changed
        .into_iter()
        .filter(|p| scope.covers_untracked() || tracked.contains_key(p))
        .filter(|p| !locally_changed.contains_key(p))
        .collect()
}

/// Pulls the latest package revision from remote and reconciles it into the
/// working tree surgically: only remote-changed tracked paths the user did not
/// touch are updated, while non-conflicting local changes are kept in place.
/// A conflicting local change on a remote-changed path blocks the whole pull.
/// Doesn't pull if there are uncommitted commits or the package has diverged.
///
/// `snapshot` carries the freshness contract: it must come from
/// [`snapshot_for_pull`], which does all network *before* the status walk, so
/// `snapshot.status` is the freshest possible input to classification.
///
/// `scope` is the caller's, never read from the lineage: this engine also backs
/// the `quilt` CLI, which passes [`SyncScope::IndividualFiles`] and so keeps
/// sparse-checkout behaviour whatever a desktop app wrote to `data.json`.
#[allow(clippy::too_many_arguments)]
pub async fn pull_package(
    lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    snapshot: PullSnapshot,
    namespace: Namespace,
    scope: SyncScope,
) -> Res<(PackageLineage, PullReport)> {
    info!("⏳ Starting pull for package {namespace} (scope={scope:?})");

    if lineage.commit.is_some() {
        error!("❌ Found pending commits, cannot pull");
        return Err(PackageOpError::Package("package has pending commits".to_string()).into());
    }

    let remote_uri = lineage.remote()?.clone();

    if remote_uri.hash != lineage.base_hash {
        error!("❌ Package has diverged from remote");
        return Err(PackageOpError::Package("package has diverged".to_string()).into());
    }

    // Defensive: `snapshot_for_pull` already short-circuits `base == latest`
    // before building the snapshot, so this never fires on the ctor-fed path.
    // It stays as a guard for hand-built snapshots (tests) and any future
    // caller that constructs the snapshot differently.
    if lineage.base_hash == lineage.latest_hash {
        error!("❌ Package is already up-to-date");
        return Err(PackageOpError::AlreadyUpToDate.into());
    }

    // `manifest` is the installed (base) manifest the caller passed in;
    // `snapshot` carries the already-fetched `latest` and its manifest.
    let outcome = classify_pull(&snapshot.status, manifest, &snapshot.latest_manifest);
    match &outcome {
        PullOutcome::UpToDate => {
            return Err(PackageOpError::AlreadyUpToDate.into());
        }
        PullOutcome::Blocked { conflicts } => {
            error!("❌ Pull blocked by conflicts: {conflicts:?}");
            return Err(PackageOpError::PullConflict(conflicts.clone()).into());
        }
        PullOutcome::CleanUpdate | PullOutcome::KeepsLocalChanges { .. } => {}
    }

    // The verify-before-uninstall pass below closes the walk→apply TOCTOU
    // window: `snapshot.status` was walked before this apply with no network in
    // between (the fetch happens before the walk, inside the ctor), so a file
    // edited after the walk is absent from `status.changes` and — if
    // remote-changed — lands in the touch-set. Re-checking the base content at
    // the destruction site turns such a raced edit into a `PullConflict`
    // instead of a silent overwrite. The residual window shrinks to the
    // verify→unlink syscalls (per file, microseconds). The one case still not
    // covered is an editor writing through an already-open fd *during* the
    // apply; that is addressed by the displace-don't-delete design in the
    // transactional-apply follow-up (the `apply_update.rs` TODO).
    //
    // TODO: this second `remote_delta` pass re-derives the partition
    // `classify_pull` just computed and discarded, and the blanket skip of
    // user-touched paths is correct only because classify already `Blocked`
    // every disagreeing both-changed path. Have the classifier return the
    // per-path disposition (or the delta) so the two derivations cannot
    // silently desynchronize.
    //
    // Kept whole rather than reduced to its keys: the per-path disposition is
    // what the report is made of, and discarding it here was why a caller could
    // learn that a package advanced but never what changed inside it.
    let delta = remote_delta(manifest, &snapshot.latest_manifest);
    let touched = touch_set(
        delta.keys().cloned(),
        &lineage.paths,
        &snapshot.status.changes,
        scope,
    );
    // Verify-before-uninstall. For every touched path, confirm the working-tree
    // file still holds the BASE content the classifier assumed — the row in
    // `manifest` (the installed/base manifest), whose self-describing
    // `ObjectHash` (a multihash) picks its own algorithm, so no `host_config`
    // is needed. `refresh_hash` returns `Ok(None)` when the file still matches
    // the base row (no drift); `Ok(Some(_))` when the content changed (drift);
    // a not-found `Err` when the file is gone (a local delete raced in — drift).
    // Any OTHER `Err` (permission denied, transient storage error) is a real
    // I/O failure, not drift: propagate it rather than reporting a recurring,
    // misleading `PullConflict`.
    //
    // This lives HERE, not inside `apply_latest_update`: `reset_to_latest`
    // shares that primitive precisely to DISCARD local drift, so a verify pass
    // there would break reset. Verify ALL paths first, then decide — never
    // interleave verification with deletion. Fail-safe in the same direction as
    // the conflict rule: the worst case is a spurious, retryable `PullConflict`,
    // never data loss.
    let mut drifted = Vec::new();
    for path in &touched {
        // Verify only what this copy actually INSTALLED. Under
        // `EntirePackage` the touch-set carries paths with no working file —
        // remote additions, and base rows never checked out (install registers
        // the manifest, not the files, so `lineage.paths` is a subset of the
        // base rows). `refresh_hash` opens the file before hashing, so a
        // not-yet-installed path would read as a not-found error, which the
        // arms below classify as drift and turn into a spurious
        // `PullConflict` aborting the whole pull. There is nothing to verify
        // for a file we never wrote: its absence is the expected state, not
        // drift.
        if !lineage.paths.contains_key(path) {
            continue;
        }
        // A tracked path always has a base row (`create_status` hard-errors
        // otherwise for a remote-backed package), so this is defensive only.
        let Some(base_row) = manifest.get_record(path) else {
            continue;
        };
        match refresh_hash(storage, &working_dir.join(path), base_row.clone()).await {
            // File still holds the base content — no drift.
            Ok(None) => {}
            // Content changed, or the file is gone (raced local delete) — drift.
            Ok(Some(_)) => drifted.push(path.clone()),
            Err(err) if err.is_not_found() => drifted.push(path.clone()),
            // A genuine I/O failure, not drift — surface it as the real error.
            Err(err) => return Err(err),
        }
    }
    if !drifted.is_empty() {
        error!("❌ Working-tree drift on touched paths since the walk: {drifted:?}");
        return Err(PackageOpError::PullConflict(drifted).into());
    }

    let latest = snapshot.latest.clone();
    let (lineage, applied) = apply_latest_update(
        lineage,
        manifest,
        paths,
        storage,
        remote,
        working_dir,
        namespace,
        snapshot.latest,
        &touched,
    )
    .await?;

    // Built after the apply, from what it did rather than from what the touch
    // set proposed — see `report_of`.
    let report = report_of(
        latest,
        &delta,
        &applied,
        &snapshot.status.changes,
        snapshot.latest_manifest.header.message.clone(),
    );

    info!("✔️ Successfully pulled (surgical), outcome={outcome:?}");
    Ok((lineage, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use aws_sdk_s3::primitives::ByteStream;
    use multihash::Multihash;

    use crate::io::remote::HostConfig;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::StorageExt;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;
    use crate::lineage::CommitState;
    use crate::lineage::PathState;
    use crate::manifest::ManifestRow;
    use crate::object_hash::Hash;
    use crate::object_hash::Sha256Hash;
    use quilt_uri::ManifestUri;
    use quilt_uri::S3Uri;

    /// A manifest row with a fake, self-describing SHA-256 multihash derived
    /// from `hash_seed` (mirrors the `row` helper in `pull_outcome.rs` tests).
    /// The digest is not a real hash of any file, so it never matches a
    /// working-tree file — exactly what the drift-detection test wants.
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

    fn hash_of(seed: &[u8]) -> crate::object_hash::ObjectHash {
        Multihash::<256>::wrap(0x12, seed)
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn uri() -> ManifestUri {
        ManifestUri {
            bucket: "b".to_string(),
            namespace: ("acme", "demo").into(),
            hash: "h".to_string(),
            origin: None,
        }
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn applied(installed: &[&str], uninstalled: &[&str]) -> Applied {
        Applied {
            installed: paths(installed),
            uninstalled: paths(uninstalled),
        }
    }

    /// The whole of the grouping: which write or deletion lands in which group.
    /// Read from the apply, so each group states something that happened.
    #[test]
    fn report_groups_by_what_happened_to_this_copy() {
        let delta = BTreeMap::from([
            (
                PathBuf::from("fetched.csv"),
                RemoteChange::Added(hash_of(b"a")),
            ),
            (
                PathBuf::from("listed.csv"),
                RemoteChange::Added(hash_of(b"b")),
            ),
            (
                PathBuf::from("changed.csv"),
                RemoteChange::Modified(hash_of(b"c")),
            ),
            (PathBuf::from("dropped.csv"), RemoteChange::Removed),
        ]);
        // `changed.csv` was tracked, so it was deleted and written again;
        // `fetched.csv` was written where there was nothing; `dropped.csv` was
        // deleted with no replacement; `listed.csv` was left on the remote.
        let applied = applied(
            &["fetched.csv", "changed.csv"],
            &["changed.csv", "dropped.csv"],
        );

        let report = report_of(
            uri(),
            &delta,
            &applied,
            &ChangeSet::new(),
            Some("a message".to_owned()),
        );

        assert_eq!(report.added, paths(&["fetched.csv"]));
        assert_eq!(report.added_not_fetched, paths(&["listed.csv"]));
        assert_eq!(report.updated, paths(&["changed.csv"]));
        assert_eq!(report.removed, paths(&["dropped.csv"]));
        assert_eq!(report.message.as_deref(), Some("a message"));
        assert!(!report.is_empty());
    }

    /// The touch set says what a pull *proposed* to move; only the apply knows
    /// what it did. Under whole-package scope the touch set covers untracked
    /// paths, and `apply_latest_update` then skips the ones with no file to
    /// delete — so a report read from the touch set claimed a deletion that
    /// never happened, and called a first fetch an update.
    #[test]
    fn untracked_paths_are_reported_by_what_the_apply_did() {
        let delta = BTreeMap::from([
            (PathBuf::from("never-had.csv"), RemoteChange::Removed),
            (
                PathBuf::from("first-fetch.csv"),
                RemoteChange::Modified(hash_of(b"m")),
            ),
        ]);
        // Both are in the touch set under whole-package scope. Neither was
        // tracked, so neither is uninstalled; the one still in `latest` is
        // written.
        let applied = applied(&["first-fetch.csv"], &[]);

        let report = report_of(uri(), &delta, &applied, &ChangeSet::new(), None);

        assert!(
            report.removed.is_empty(),
            "a path with no local file was reported as removed: {report:?}"
        );
        assert_eq!(
            report.added,
            paths(&["first-fetch.csv"]),
            "a first fetch is new to this copy, not an update"
        );
        assert!(report.updated.is_empty(), "nothing was overwritten");
    }

    /// The dispositions that must produce **nothing**, because nothing moved.
    /// Reporting any of them would tell the user a file changed when it did not.
    #[test]
    fn nothing_moved_means_nothing_reported() {
        let nothing = Applied::default();

        // A tracked path the remote changed and the user had also edited: the
        // touch set drops it, their work is kept, the remote's version is not
        // applied — so it is neither "updated" nor a skip notice.
        let kept = BTreeMap::from([(
            PathBuf::from("mine.csv"),
            RemoteChange::Modified(hash_of(b"x")),
        )]);
        let report = report_of(uri(), &kept, &nothing, &ChangeSet::new(), None);
        assert!(
            report.is_empty(),
            "kept local work was reported: {report:?}"
        );

        // A path removed on both sides — trivially resolved, never touched.
        let both_removed = BTreeMap::from([(PathBuf::from("gone.csv"), RemoteChange::Removed)]);
        let report = report_of(uri(), &both_removed, &nothing, &ChangeSet::new(), None);
        assert!(
            report.is_empty(),
            "a both-removed path was reported: {report:?}"
        );

        // A path both sides added with the same content: the classifier lets
        // the pull through, and the file is already on disk. Calling it "not
        // downloaded" would send the user looking for something they have.
        let both_added = BTreeMap::from([(
            PathBuf::from("same.csv"),
            RemoteChange::Added(hash_of(b"same")),
        )]);
        let locally = ChangeSet::from([(
            PathBuf::from("same.csv"),
            Change::Added(ManifestRow::default()),
        )]);
        let report = report_of(uri(), &both_added, &nothing, &locally, None);
        assert!(
            report.is_empty(),
            "a path the user already has was reported as outstanding: {report:?}"
        );

        // A metadata-only revision: identical rows, so an empty delta and
        // nothing applied. The hashes advance and no file moved.
        let report = report_of(
            uri(),
            &BTreeMap::new(),
            &nothing,
            &ChangeSet::new(),
            Some("retagged".to_owned()),
        );
        assert!(report.is_empty(), "a metadata-only revision named files");
        // Its message still travels — it is the only thing that changed.
        assert_eq!(report.message.as_deref(), Some("retagged"));
    }

    /// `ManifestHeader::default` writes `Some(String::new())`, which the desktop
    /// already treats as absent elsewhere.
    #[test]
    fn an_empty_message_is_absent() {
        let report = report_of(
            uri(),
            &BTreeMap::new(),
            &Applied::default(),
            &ChangeSet::new(),
            Some(String::new()),
        );
        assert_eq!(report.message, None);
    }

    fn manifest_of(rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            rows,
            ..Manifest::default()
        }
    }

    /// A hand-built snapshot for the guard tests: real `status`, dummy
    /// `latest`/`latest_manifest`. Guards that fire before classification never
    /// look at the manifests; where classification is reached, the test picks
    /// the manifests deliberately.
    fn snapshot_with(status: InstalledPackageStatus, latest_manifest: Manifest) -> PullSnapshot {
        PullSnapshot {
            status,
            latest: ManifestUri::default(),
            latest_manifest,
        }
    }

    // Gentle pull no longer refuses on a working-tree change: an added file
    // that the remote did not touch is kept, and pull proceeds. (Behind + one
    // added file — Kevin's field report.) Full end-to-end apply is covered by
    // the primitive's tests; here we assert the guard is *gone* by getting past
    // it to the classify `UpToDate` arm (identical base/latest manifests) even
    // with a local add present.
    #[test(tokio::test)]
    async fn added_file_does_not_block_the_guard() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        // base != latest so the defensive `base == latest` guard does NOT fire;
        // remote hash == base so it is not diverged. Classification then runs on
        // identical base/latest manifests and returns `UpToDate`.
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            }),
            base_hash: "a".to_string(),
            latest_hash: "b".to_string(),
            ..PackageLineage::default()
        };
        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("new"),
                Change::Added(ManifestRow::default()),
            )]),
            ..InstalledPackageStatus::default()
        };
        // Identical base (the `manifest` arg) and latest manifests → the
        // classifier returns `UpToDate`, which pull maps to `AlreadyUpToDate`.
        let error = pull_package(
            lineage,
            &mut Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            snapshot_with(status, Manifest::default()),
            Namespace::default(),
            SyncScope::IndividualFiles,
        )
        .await;
        // Reaches the up-to-date branch (guard relaxed), not "pending changes".
        assert!(matches!(
            error.unwrap_err(),
            crate::Error::PackageOp(PackageOpError::AlreadyUpToDate)
        ));
    }

    // The ctor short-circuits when the resolved `latest` tag already equals
    // `base_hash`: it returns `AlreadyUpToDate` WITHOUT fetching the manifest.
    #[test(tokio::test)]
    async fn snapshot_short_circuits_when_latest_equals_base() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let bucket = "bkt";
        let base = "base";
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                bucket: bucket.to_string(),
                namespace: ("f", "b").into(),
                hash: base.to_string(),
                origin: None,
            }),
            base_hash: base.to_string(),
            latest_hash: base.to_string(),
            ..PackageLineage::default()
        };
        // Stage the `latest` tag so it resolves back to `base`.
        let tag_uri =
            S3Uri::try_from(format!("s3://{bucket}/.quilt/named_packages/f/b/latest").as_str())
                .unwrap();
        remote
            .put_object(None, &tag_uri, base.as_bytes().to_vec())
            .await
            .unwrap();

        let result = snapshot_for_pull(
            lineage,
            &Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            HostConfig::default(),
        )
        .await;

        assert!(matches!(
            result.unwrap_err(),
            crate::Error::PackageOp(PackageOpError::AlreadyUpToDate)
        ));
        // The manifest was never fetched — the short-circuit fired first.
        let manifest_uri = format!("s3://{bucket}/.quilt/packages/{base}");
        assert_eq!(remote.get_object_count(&manifest_uri), 0);
    }

    #[test(tokio::test)]
    async fn test_no_pull_if_commit() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            ..PackageLineage::default()
        };

        let error = pull_package(
            lineage,
            &mut Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            snapshot_with(InstalledPackageStatus::default(), Manifest::default()),
            Namespace::default(),
            SyncScope::IndividualFiles,
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package has pending commits".to_string()
        );
    }

    #[test(tokio::test)]
    async fn test_no_pull_if_diverged() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            }),
            base_hash: "b".to_string(),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &mut Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            snapshot_with(InstalledPackageStatus::default(), Manifest::default()),
            Namespace::default(),
            SyncScope::IndividualFiles,
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package has diverged".to_string()
        );
    }

    #[test(tokio::test)]
    async fn test_no_pull_if_up_to_date() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            }),
            base_hash: "a".to_string(),
            latest_hash: "a".to_string(),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &mut Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            snapshot_with(InstalledPackageStatus::default(), Manifest::default()),
            Namespace::default(),
            SyncScope::IndividualFiles,
        )
        .await;
        assert!(matches!(
            error.unwrap_err(),
            crate::Error::PackageOp(PackageOpError::AlreadyUpToDate)
        ));
    }

    // Verify-before-uninstall closes the walk→apply TOCTOU window. The walk saw
    // an EMPTY changeset (stale snapshot), the remote changed `a`, so `a` lands
    // in the touch-set. But the working-tree file at `a` was edited AFTER the
    // walk. The verify pass must catch the drift and abort as `PullConflict`
    // before any file is uninstalled — never silently overwrite the raced edit.
    #[test(tokio::test)]
    async fn racing_edit_on_touched_path_aborts_as_conflict() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let working_dir = PathBuf::from("/wd");
        let path = PathBuf::from("a");

        // The raced edit: working-tree content that does NOT match the base row
        // the classifier assumed for `a`.
        let edited = b"raced edit after the walk";
        storage
            .write_byte_stream(working_dir.join(&path), ByteStream::from_static(edited))
            .await
            .unwrap();

        // Base tracks `a` at the v1 hash; the remote changed it to v2, so `a`
        // is a remote-changed tracked path → it enters the touch-set.
        let base = manifest_of(vec![row("a", b"v1")]);
        let latest = manifest_of(vec![row("a", b"v2")]);

        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            }),
            base_hash: "a".to_string(),
            latest_hash: "b".to_string(),
            paths: BTreeMap::from([(path.clone(), PathState::default())]),
            ..PackageLineage::default()
        };
        // Stale snapshot: the walk observed NO changes (the edit raced in after).
        let status = InstalledPackageStatus::default();

        let mut base = base;
        let error = pull_package(
            lineage,
            &mut base,
            &DomainPaths::default(),
            &storage,
            &remote,
            working_dir.clone(),
            snapshot_with(status, latest),
            Namespace::default(),
            SyncScope::IndividualFiles,
        )
        .await;

        assert!(
            matches!(
                error.as_ref().unwrap_err(),
                crate::Error::PackageOp(PackageOpError::PullConflict(paths)) if paths == &vec![path.clone()]
            ),
            "expected PullConflict naming `a`, got: {error:?}"
        );

        // Nothing was deleted or overwritten — the raced edit is intact.
        assert_eq!(
            storage.read_bytes(&working_dir.join(&path)).await.unwrap(),
            edited
        );
    }

    // A touched path whose working-tree file is GONE (a local delete raced in
    // after the walk) is drift, not a real I/O error: `refresh_hash` surfaces a
    // not-found `Err`, which the verify pass folds into the drifted set and
    // reports as a retryable `PullConflict` — never a propagated failure.
    #[test(tokio::test)]
    async fn missing_touched_path_aborts_as_conflict() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let working_dir = PathBuf::from("/wd");
        let path = PathBuf::from("a");

        // No working-tree file is written for `a`: `refresh_hash` → `open_file`
        // fails with a not-found error.

        // Base tracks `a`; the remote changed it → `a` enters the touch-set.
        let base = manifest_of(vec![row("a", b"v1")]);
        let latest = manifest_of(vec![row("a", b"v2")]);

        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            }),
            base_hash: "a".to_string(),
            latest_hash: "b".to_string(),
            paths: BTreeMap::from([(path.clone(), PathState::default())]),
            ..PackageLineage::default()
        };
        // Stale snapshot: the walk observed NO changes (the delete raced in).
        let status = InstalledPackageStatus::default();

        let mut base = base;
        let error = pull_package(
            lineage,
            &mut base,
            &DomainPaths::default(),
            &storage,
            &remote,
            working_dir.clone(),
            snapshot_with(status, latest),
            Namespace::default(),
            SyncScope::IndividualFiles,
        )
        .await;

        assert!(
            matches!(
                error.as_ref().unwrap_err(),
                crate::Error::PackageOp(PackageOpError::PullConflict(paths)) if paths == &vec![path.clone()]
            ),
            "expected PullConflict naming `a`, got: {error:?}"
        );
    }

    fn tracked_map<const N: usize>(names: [&str; N]) -> LineagePaths {
        names
            .iter()
            .map(|n| (PathBuf::from(n), PathState::default()))
            .collect()
    }

    /// **The scope, both directions.** A path the remote added is fetched under
    /// whole-package scope and left alone under the narrow one. Asserted both
    /// ways round on the same input, so this cannot pass by the two scopes
    /// behaving alike — which is exactly how a broken filter would look.
    #[test]
    fn a_remote_added_path_is_taken_only_under_whole_package_scope() {
        let remote_changed = || vec![PathBuf::from("have.csv"), PathBuf::from("added.csv")];
        let tracked = tracked_map(["have.csv"]);
        let none = ChangeSet::new();

        assert_eq!(
            touch_set(
                remote_changed(),
                &tracked,
                &none,
                SyncScope::IndividualFiles
            ),
            vec![PathBuf::from("have.csv")],
            "sparse checkout leaves a path it does not track"
        );
        assert_eq!(
            touch_set(remote_changed(), &tracked, &none, SyncScope::EntirePackage),
            vec![PathBuf::from("have.csv"), PathBuf::from("added.csv")],
            "whole-package scope takes it"
        );
    }

    /// A base row this copy never installed is in the same boat as a remote
    /// addition: untracked, so narrow scope skips it and whole-package scope
    /// picks it up. This is the case that reaches the verify pass with no file
    /// on disk — see `a_never_installed_path_does_not_abort_the_verify_pass`.
    #[test]
    fn a_never_installed_path_follows_the_same_rule() {
        let never = vec![PathBuf::from("never-installed.csv")];
        let tracked = tracked_map(["something-else.csv"]);
        let none = ChangeSet::new();

        assert!(touch_set(never.clone(), &tracked, &none, SyncScope::IndividualFiles).is_empty());
        assert_eq!(
            touch_set(never.clone(), &tracked, &none, SyncScope::EntirePackage),
            never
        );
    }

    /// Widening what a pull *fetches* must not widen what it *overwrites*: a
    /// path the user changed is dropped under **both** scopes. The classifier
    /// has already blocked anything that genuinely disagrees, so what survives
    /// to here is local work to keep.
    #[test]
    fn a_locally_changed_path_is_dropped_under_either_scope() {
        let remote_changed = || vec![PathBuf::from("mine.csv")];
        let tracked = tracked_map(["mine.csv"]);
        let mine = ChangeSet::from([(
            PathBuf::from("mine.csv"),
            Change::Modified(ManifestRow::default()),
        )]);

        for scope in [SyncScope::IndividualFiles, SyncScope::EntirePackage] {
            assert!(
                touch_set(remote_changed(), &tracked, &mine, scope).is_empty(),
                "{scope:?} must not overwrite the user's own change"
            );
        }
    }

    /// **The regression the verify re-gate exists for.** Under whole-package
    /// scope the touch-set carries paths with no working file — here a base row
    /// that was never installed, which the remote has since removed. The verify
    /// pass hashes a path's file to check it still holds the base content;
    /// against a file that was never written, that read fails as not-found, and
    /// before the re-gate it was classified as drift and aborted the whole pull
    /// with a spurious `PullConflict`. Nothing to verify is not drift.
    #[test(tokio::test)]
    async fn a_never_installed_path_does_not_abort_the_verify_pass() -> crate::Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let bucket = "bkt";
        let namespace: Namespace = ("f", "b").into();
        let (base_hash, latest_hash) = ("OLD", "NEW");
        let working_dir = PathBuf::from("/wd");
        let never = PathBuf::from("never-installed.csv");

        let paths = DomainPaths::default();
        paths.scaffold_for_caching(&storage, bucket).await?;

        // `never` has a base row but no working file and no lineage entry —
        // install registers the manifest, not the files. `latest` drops it, so
        // it lands in the touch-set as a removal and the apply has nothing to
        // download.
        let base = manifest_of(vec![ManifestRow {
            logical_key: never.clone(),
            ..ManifestRow::default()
        }]);
        let latest_manifest = manifest_of(vec![]);
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://{bucket}/.quilt/packages/{latest_hash}").as_str())?,
                br#"{"version": "v0"}"#.to_vec(),
            )
            .await?;

        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                bucket: bucket.to_string(),
                namespace: namespace.clone(),
                hash: base_hash.to_string(),
                origin: None,
            }),
            base_hash: base_hash.to_string(),
            latest_hash: latest_hash.to_string(),
            // Deliberately empty: nothing about this package is installed.
            paths: BTreeMap::new(),
            ..PackageLineage::default()
        };
        let snapshot = PullSnapshot {
            status: InstalledPackageStatus::default(),
            latest: ManifestUri {
                bucket: bucket.to_string(),
                namespace: namespace.clone(),
                hash: latest_hash.to_string(),
                origin: None,
            },
            latest_manifest,
        };

        let mut base = base;
        let (lineage, _report) = pull_package(
            lineage,
            &mut base,
            &paths,
            &storage,
            &remote,
            working_dir,
            snapshot,
            namespace,
            SyncScope::EntirePackage,
        )
        .await?;

        assert_eq!(
            lineage.base_hash, latest_hash,
            "the pull completed instead of reporting drift on a file that was never written"
        );
        Ok(())
    }

    // The happy counterpart: a touched path whose working-tree file still holds
    // the BASE content the classifier assumed passes the verify pass, so the
    // pull proceeds through `apply_latest_update` to success. Uses a
    // remote-REMOVED touched path so the apply only uninstalls (no object
    // downloads to stage).
    #[test(tokio::test)]
    async fn touched_path_matching_base_passes_verify_and_pulls() -> crate::Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let bucket = "bkt";
        let namespace: Namespace = ("f", "b").into();
        let base_hash = "OLD";
        let latest_hash = "NEW";
        let working_dir = PathBuf::from("/wd");
        let path = PathBuf::from("a");

        let paths = DomainPaths::default();
        paths.scaffold_for_caching(&storage, bucket).await?;

        // Working-tree file for `a`, and the base row carrying its REAL hash so
        // the verify pass sees no drift.
        let content = b"the base content of a";
        storage
            .write_byte_stream(working_dir.join(&path), ByteStream::from_static(content))
            .await?;
        let file = storage.open_file(&working_dir.join(&path)).await?;
        let hash: Multihash<256> = Sha256Hash::from_reader(file, content.len() as u64)
            .await?
            .into();
        let base_row = ManifestRow {
            logical_key: path.clone(),
            hash: hash.try_into()?,
            size: content.len() as u64,
            ..ManifestRow::default()
        };
        let base = manifest_of(vec![base_row]);

        // `latest` drops `a` (remote removal): base != latest → CleanUpdate; the
        // touch-set is {`a`: Removed}; apply only uninstalls it.
        let latest_manifest = manifest_of(vec![]);
        let latest_uri = ManifestUri {
            bucket: bucket.to_string(),
            namespace: namespace.clone(),
            hash: latest_hash.to_string(),
            origin: None,
        };
        // The apply re-fetches the `latest` manifest from the remote.
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://{bucket}/.quilt/packages/{latest_hash}").as_str())?,
                br#"{"version": "v0"}"#.to_vec(),
            )
            .await?;

        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                bucket: bucket.to_string(),
                namespace: namespace.clone(),
                hash: base_hash.to_string(),
                origin: None,
            }),
            base_hash: base_hash.to_string(),
            latest_hash: latest_hash.to_string(),
            paths: BTreeMap::from([(path.clone(), PathState::default())]),
            ..PackageLineage::default()
        };
        let snapshot = PullSnapshot {
            status: InstalledPackageStatus::default(),
            latest: latest_uri,
            latest_manifest,
        };

        let mut base = base;
        let (lineage, _report) = pull_package(
            lineage,
            &mut base,
            &paths,
            &storage,
            &remote,
            working_dir.clone(),
            snapshot,
            namespace,
            SyncScope::IndividualFiles,
        )
        .await?;

        // Pull advanced to `latest` and uninstalled the remote-removed path.
        assert_eq!(lineage.base_hash, latest_hash);
        assert!(!lineage.paths.contains_key(&path));
        Ok(())
    }
}
