use std::path::Path;
use std::path::PathBuf;

use tracing::error;
use tracing::info;

use crate::Res;
use crate::error::PackageOpError;
use crate::flow;
use crate::flow::PullOutcome;
use crate::flow::apply_latest_update;
use crate::flow::classify_pull;
use crate::flow::remote_delta;
use crate::io::manifest::resolve_tag;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
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

/// Pulls the latest package revision from remote and reconciles it into the
/// working tree surgically: only remote-changed tracked paths the user did not
/// touch are updated, while non-conflicting local changes are kept in place.
/// A conflicting local change on a remote-changed path blocks the whole pull.
/// Doesn't pull if there are uncommitted commits or the package has diverged.
///
/// `snapshot` carries the freshness contract: it must come from
/// [`snapshot_for_pull`], which does all network *before* the status walk, so
/// `snapshot.status` is the freshest possible input to classification.
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
) -> Res<PackageLineage> {
    info!("⏳ Starting pull for package {}", namespace);

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

    // TODO: `snapshot.status` is walked before the apply below with no network
    // in between (the fetch now happens before the walk, inside the ctor), so
    // the remaining window is walk→apply: a file edited after the walk but
    // before the apply is absent from `status.changes`, lands in the touch-set
    // if remote-changed, and is overwritten with no conflict signal. Close it
    // with a verify-before-uninstall pass in `apply_latest_update`.
    //
    // TODO: this second `remote_delta` pass re-derives the partition
    // `classify_pull` just computed and discarded, and the blanket skip of
    // user-touched paths is correct only because classify already `Blocked`
    // every disagreeing both-changed path. Have the classifier return the
    // per-path disposition (or the delta) so the two derivations cannot
    // silently desynchronize.
    //
    // Touch-set: remote-changed tracked paths the user did NOT touch. Paths the
    // user changed are left in place (kept, or trivially resolved).
    let touched: Vec<PathBuf> = remote_delta(manifest, &snapshot.latest_manifest)
        .into_keys()
        .filter(|p| lineage.paths.contains_key(p))
        .filter(|p| !snapshot.status.changes.contains_key(p))
        .collect();

    let lineage = apply_latest_update(
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

    info!("✔️ Successfully pulled (surgical), outcome={outcome:?}");
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use crate::io::remote::HostConfig;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;
    use crate::lineage::CommitState;
    use crate::manifest::ManifestRow;
    use quilt_uri::ManifestUri;
    use quilt_uri::S3Uri;

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
        )
        .await;
        assert!(matches!(
            error.unwrap_err(),
            crate::Error::PackageOp(PackageOpError::AlreadyUpToDate)
        ));
    }
}
