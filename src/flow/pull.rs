use std::path::PathBuf;

use crate::flow::browse::cache_remote_manifest;
use crate::flow::install_paths::install_paths;
use crate::flow::status::InstalledPackageStatus;
use crate::flow::uninstall_paths::uninstall_paths;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::quilt::manifest_handle;
use crate::quilt::uri::Namespace;
use crate::s3_utils;
use crate::Error;

pub async fn pull_package(
    lineage: PackageLineage,
    manifest: &(impl manifest_handle::ReadableManifest + Sync),
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    working_dir: PathBuf,
    status: InstalledPackageStatus,
    namespace: Namespace,
) -> Result<PackageLineage, Error> {
    if !status.changes.is_empty() {
        return Err(Error::Package("package has pending changes".to_string()));
    }

    if lineage.commit.is_some() {
        return Err(Error::Package("package has pending commits".to_string()));
    }
    if lineage.remote.hash != lineage.base_hash {
        return Err(Error::Package("package has diverged".to_string()));
    }
    // TODO: do we need to explicitly update latest_hash?
    // status() tries to update, but may fail.
    if lineage.base_hash == lineage.latest_hash {
        return Err(Error::Package("package is already up-to-date".to_string()));
    }

    // TODO: What should we do about installed paths?
    // They may or may not exist in the updated package.
    let installed_paths: Vec<PathBuf> = lineage.paths.keys().cloned().collect();
    let mut lineage =
        uninstall_paths(lineage, working_dir.clone(), storage, &installed_paths).await?;

    // TODO: uninstall_paths() just modified the lineage, so re-reading it here.
    // There needs to be a better way.
    lineage.remote.hash = lineage.latest_hash.clone();
    lineage.base_hash = lineage.latest_hash.clone();

    // FIXME: pass from outside
    let remote = s3_utils::RemoteS3::new();
    cache_remote_manifest(paths, storage, &remote, &lineage.remote).await?;
    copy_cached_to_installed(
        paths,
        storage,
        &lineage.remote.bucket,
        &namespace,
        &lineage.remote.hash,
    )
    .await?;

    let materialized_manifest = manifest.read(storage).await?;
    let paths_to_install = installed_paths
        .into_iter()
        .filter(|x| materialized_manifest.records.contains_key(x))
        .collect();
    install_paths(
        lineage,
        manifest,
        paths,
        working_dir,
        namespace,
        storage,
        &remote,
        &paths_to_install,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::flow::status::Change;
    use crate::flow::status::DiscreteChange;
    use crate::io::storage::mocks::MockStorage;
    use crate::quilt::mocks;
    use crate::quilt::RemoteManifest;

    #[tokio::test]
    async fn test_no_pull_if_changes() -> Result<(), Error> {
        let storage = MockStorage::default();
        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("a/a")]);

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("foo"),
                Change {
                    previous: None,
                    current: None,
                    state: DiscreteChange::Pristine,
                },
            )]),
            ..InstalledPackageStatus::default()
        };
        let error = pull_package(
            lineage,
            &mocks::manifest::with_record_keys(vec![PathBuf::from("a/a")]),
            &DomainPaths::default(),
            &storage,
            PathBuf::default(),
            status,
            Namespace::default(),
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package has pending changes".to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_no_pull_if_commit() {
        let storage = MockStorage::default();
        let lineage = mocks::lineage::with_commit();
        let error = pull_package(
            lineage,
            &mocks::manifest::default(),
            &DomainPaths::default(),
            &storage,
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

    #[tokio::test]
    async fn test_no_pull_if_diverged() {
        let storage = MockStorage::default();
        let lineage = PackageLineage {
            remote: RemoteManifest {
                hash: "a".to_string(),
                ..RemoteManifest::default()
            },
            base_hash: "b".to_string(),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &mocks::manifest::default(),
            &DomainPaths::default(),
            &storage,
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

    #[tokio::test]
    async fn test_no_pull_if_up_to_date() {
        let storage = MockStorage::default();
        let lineage = PackageLineage {
            remote: RemoteManifest {
                hash: "a".to_string(),
                ..RemoteManifest::default()
            },
            base_hash: "a".to_string(),
            latest_hash: "a".to_string(),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &mocks::manifest::default(),
            &DomainPaths::default(),
            &storage,
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
