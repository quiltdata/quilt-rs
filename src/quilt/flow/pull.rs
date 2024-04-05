use std::path::PathBuf;

use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::quilt::flow::browse::cache_remote_manifest;
use crate::quilt::flow::install_paths::install_paths;
use crate::quilt::flow::status::create_status;
use crate::quilt::flow::uninstall_paths::uninstall_paths;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::manifest_handle;
use crate::quilt::storage::Storage;
use crate::quilt::Error;
use crate::s3_utils;

pub async fn pull_package(
    lineage: PackageLineage,
    manifest: &(impl manifest_handle::ReadableManifest + Sync),
    paths: &DomainPaths,
    storage: &mut impl Storage,
    working_dir: PathBuf,
    namespace: String,
) -> Result<PackageLineage, Error> {
    let (lineage, status) = create_status(lineage, storage, manifest, working_dir.clone()).await?;
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
    let installed_paths: Vec<String> = lineage.paths.keys().cloned().collect();
    let mut lineage =
        uninstall_paths(lineage, working_dir.clone(), storage, &installed_paths).await?;

    // TODO: uninstall_paths() just modified the lineage, so re-reading it here.
    // There needs to be a better way.
    lineage.remote.hash = lineage.latest_hash.clone();
    lineage.base_hash = lineage.latest_hash.clone();

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

    let materialized_manifest = manifest.read().await?;
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
        &paths_to_install,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::quilt::lineage::CommitState;
    use crate::quilt::lineage::PathState;
    use crate::quilt::manifest_handle::ReadableManifest;
    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::quilt::RemoteManifest;
    use crate::Row4;
    use crate::Table;

    struct InMemoryManifest {}
    impl ReadableManifest for InMemoryManifest {
        async fn read(&self) -> Result<Table, Error> {
            Ok(Table::default())
        }
    }

    #[tokio::test]
    async fn test_no_pull_if_changes() {
        let mut storage = MockStorage::default();
        let lineage = PackageLineage {
            paths: BTreeMap::from([("a/a".to_string(), PathState::default())]),
            ..PackageLineage::default()
        };
        struct RemovedFilesManifest {}
        impl ReadableManifest for RemovedFilesManifest {
            async fn read(&self) -> Result<Table, Error> {
                Ok(Table {
                    records: BTreeMap::from([("a/a".to_string(), Row4::default())]),
                    ..Table::default()
                })
            }
        }

        let error = pull_package(
            lineage,
            &(RemovedFilesManifest {}),
            &DomainPaths::default(),
            &mut storage,
            PathBuf::default(),
            String::default(),
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package has pending changes".to_string()
        );
    }

    #[tokio::test]
    async fn test_no_pull_if_commit() {
        let mut storage = MockStorage::default();
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &(InMemoryManifest {}),
            &DomainPaths::default(),
            &mut storage,
            PathBuf::default(),
            String::default(),
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package has pending commits".to_string()
        );
    }

    #[tokio::test]
    async fn test_no_pull_if_diverged() {
        let mut storage = MockStorage::default();
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
            &(InMemoryManifest {}),
            &DomainPaths::default(),
            &mut storage,
            PathBuf::default(),
            String::default(),
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package has diverged".to_string()
        );
    }

    #[tokio::test]
    async fn test_no_pull_if_up_to_date() {
        let mut storage = MockStorage::default();
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
            &(InMemoryManifest {}),
            &DomainPaths::default(),
            &mut storage,
            PathBuf::default(),
            String::default(),
        )
        .await;
        assert_eq!(
            error.unwrap_err().to_string(),
            "General error regarding package: package is already up-to-date".to_string()
        );
    }
}
