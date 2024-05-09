use std::path::PathBuf;

use crate::flow::browse::cache_remote_manifest;
use crate::flow::install_paths::install_paths;
use crate::flow::uninstall_paths::uninstall_paths;
use crate::io::manifest::resolve_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Table;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::Error;

#[allow(clippy::too_many_arguments)]
pub async fn pull_package(
    lineage: PackageLineage,
    manifest: &mut Table,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
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

    let manifest_uri = resolve_latest(remote, lineage.remote.clone().into()).await?;
    cache_remote_manifest(paths, storage, remote, &manifest_uri).await?;
    copy_cached_to_installed(
        paths,
        storage,
        &ManifestUri {
            namespace: namespace.clone(),
            ..lineage.remote.clone()
        },
    )
    .await?;

    let mut paths_to_install = Vec::new();
    for x in installed_paths {
        if manifest.contains_record(&x).await {
            paths_to_install.push(x)
        }
    }
    install_paths(
        lineage,
        manifest,
        paths,
        working_dir,
        namespace,
        storage,
        remote,
        &paths_to_install,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::lineage::Change;
    use crate::lineage::DiscreteChange;
    use crate::mocks;

    #[tokio::test]
    async fn test_no_pull_if_changes() -> Result<(), Error> {
        let storage = mocks::storage::MockStorage::default();
        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("a/a")]);

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("foo"),
                Change {
                    previous: None,
                    current: None,
                    state: DiscreteChange::Added,
                },
            )]),
            ..InstalledPackageStatus::default()
        };
        let remote = mocks::remote::MockRemote::default();
        let error = pull_package(
            lineage,
            &mut mocks::manifest::with_record_keys(vec![PathBuf::from("a/a")]),
            &DomainPaths::default(),
            &storage,
            &remote,
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
        let storage = mocks::storage::MockStorage::default();
        let remote = mocks::remote::MockRemote::default();
        let lineage = mocks::lineage::with_commit();
        let error = pull_package(
            lineage,
            &mut Table::default(),
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

    #[tokio::test]
    async fn test_no_pull_if_diverged() {
        let storage = mocks::storage::MockStorage::default();
        let remote = mocks::remote::MockRemote::default();
        let lineage = PackageLineage {
            remote: ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            },
            base_hash: "b".to_string(),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &mut Table::default(),
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

    #[tokio::test]
    async fn test_no_pull_if_up_to_date() {
        let storage = mocks::storage::MockStorage::default();
        let remote = mocks::remote::MockRemote::default();
        let lineage = PackageLineage {
            remote: ManifestUri {
                hash: "a".to_string(),
                ..ManifestUri::default()
            },
            base_hash: "a".to_string(),
            latest_hash: "a".to_string(),
            ..PackageLineage::default()
        };
        let error = pull_package(
            lineage,
            &mut Table::default(),
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
