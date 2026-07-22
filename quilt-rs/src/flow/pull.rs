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
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;
use quilt_uri::Tag;

/// Pulls the latest package revision from remote and reconciles it into the
/// working tree surgically: only remote-changed tracked paths the user did not
/// touch are updated, while non-conflicting local changes are kept in place.
/// A conflicting local change on a remote-changed path blocks the whole pull.
/// Doesn't pull if there are uncommitted commits or the package has diverged.
#[allow(clippy::too_many_arguments)]
pub async fn pull_package(
    lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    status: InstalledPackageStatus,
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

    // TODO: do we need to explicitly update latest_hash?
    // status() tries to update, but may fail.
    if lineage.base_hash == lineage.latest_hash {
        error!("❌ Package is already up-to-date");
        return Err(PackageOpError::Package("package is already up-to-date".to_string()).into());
    }

    // Resolve + cache the `latest` manifest, then classify before mutating.
    let remote_uri = lineage.remote()?.clone();
    let origin = remote_uri.origin.clone();
    let latest = resolve_tag(remote, origin.as_ref(), remote_uri, Tag::Latest).await?;
    let latest_manifest = flow::cache_remote_manifest(paths, storage, remote, &latest).await?;

    // `manifest` is the installed (base) manifest the caller passed in.
    let outcome = classify_pull(&status, manifest, &latest_manifest);
    match &outcome {
        PullOutcome::UpToDate => {
            return Err(
                PackageOpError::Package("package is already up-to-date".to_string()).into(),
            );
        }
        PullOutcome::Blocked { conflicts } => {
            error!("❌ Pull blocked by conflicts: {conflicts:?}");
            return Err(PackageOpError::PullConflict(conflicts.clone()).into());
        }
        PullOutcome::CleanUpdate | PullOutcome::KeepsLocalChanges { .. } => {}
    }

    // Touch-set: remote-changed tracked paths the user did NOT touch. Paths the
    // user changed are left in place (kept, or trivially resolved).
    let touched: Vec<PathBuf> = remote_delta(manifest, &latest_manifest)
        .into_keys()
        .filter(|p| lineage.paths.contains_key(p))
        .filter(|p| !status.changes.contains_key(p))
        .collect();

    let lineage = apply_latest_update(
        lineage,
        manifest,
        paths,
        storage,
        remote,
        working_dir,
        namespace,
        latest,
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

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;
    use crate::lineage::CommitState;
    use crate::manifest::ManifestRow;
    use quilt_uri::ManifestUri;

    // Gentle pull no longer refuses on a working-tree change: an added file
    // that the remote did not touch is kept, and pull proceeds. (Behind + one
    // added file — Kevin's field report.) Full end-to-end apply is covered by
    // the primitive's tests; here we assert the guard is *gone* by getting past
    // it to the up-to-date short-circuit when base == latest with local changes.
    #[test(tokio::test)]
    async fn added_file_does_not_block_the_guard() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        // base == latest → up-to-date short-circuit, but only reached if the
        // working-tree guard no longer fires first.
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            }),
            base_hash: "a".to_string(),
            latest_hash: "a".to_string(),
            ..PackageLineage::default()
        };
        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("new"),
                Change::Added(ManifestRow::default()),
            )]),
            ..InstalledPackageStatus::default()
        };
        let error = pull_package(
            lineage,
            &mut Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            status,
            Namespace::default(),
        )
        .await;
        // Reaches the up-to-date branch (guard relaxed), not "pending changes".
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package is already up-to-date".to_string()
        );
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
            InstalledPackageStatus::default(),
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
            InstalledPackageStatus::default(),
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
            InstalledPackageStatus::default(),
            Namespace::default(),
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package is already up-to-date".to_string()
        );
    }
}
