use std::path::PathBuf;

use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

use crate::flow;
use crate::io::manifest::resolve_tag;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Table;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::Tag;
use crate::Error;
use crate::Res;

/// Pulls the latest package from remote.
/// It also remove every local file in working directory and then re-installs it.
/// Doesn't pull if there are uncommited changes in working directory.
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
) -> Res<PackageLineage> {
    info!("⏳ Starting pull for package {}", namespace);

    if !status.changes.is_empty() {
        error!("❌ Found pending changes, cannot pull");
        return Err(Error::Package("package has pending changes".to_string()));
    }

    if lineage.commit.is_some() {
        error!("❌ Found pending commits, cannot pull");
        return Err(Error::Package("package has pending commits".to_string()));
    }

    if lineage.remote.hash != lineage.base_hash {
        error!("❌ Package has diverged from remote");
        return Err(Error::Package("package has diverged".to_string()));
    }

    // TODO: do we need to explicitly update latest_hash?
    // status() tries to update, but may fail.
    if lineage.base_hash == lineage.latest_hash {
        error!("❌ Package is already up-to-date");
        return Err(Error::Package("package is already up-to-date".to_string()));
    }

    // TODO: What should we do about installed paths?
    // They may or may not exist in the updated package.
    let installed_paths: Vec<PathBuf> = lineage.paths.keys().cloned().collect();
    debug!("⏳ Uninstalling {} paths", installed_paths.len());
    let mut lineage =
        flow::uninstall_paths(lineage, working_dir.clone(), storage, &installed_paths).await?;

    debug!("⏳ Updating lineage hashes");
    // TODO: uninstall_paths() just modified the lineage, so re-reading it here.
    // There needs to be a better way.
    lineage.remote.hash.clone_from(&lineage.latest_hash);
    lineage.base_hash.clone_from(&lineage.latest_hash);

    debug!("⏳ Resolving latest manifest");
    let manifest_uri = resolve_tag(
        remote,
        &lineage.remote.catalog,
        &lineage.remote.clone().into(),
        Tag::Latest,
    )
    .await?;
    debug!("✔️ Latest manifest resolved: {}", manifest_uri.display());

    debug!("⏳ Caching remote manifest");
    flow::cache_remote_manifest(paths, storage, remote, &manifest_uri).await?;

    debug!("⏳ Installing cached manifest");
    copy_cached_to_installed(
        paths,
        storage,
        &ManifestUri {
            namespace: namespace.clone(),
            ..lineage.remote.clone()
        },
    )
    .await?;

    debug!("⏳ Checking which paths to reinstall");
    let mut paths_to_install = Vec::new();
    for x in &installed_paths {
        if manifest.contains_record(x).await {
            debug!("✔️ Will reinstall path: {}", x.display());
            paths_to_install.push(x)
        } else {
            warn!("❌ Path no longer exists in manifest: {}", x.display());
        }
    }
    info!("⏳ Reinstalling {} paths", paths_to_install.len());
    let package_lineage = flow::install_paths(
        lineage,
        manifest,
        paths,
        working_dir,
        namespace,
        storage,
        remote,
        &paths_to_install,
    )
    .await?;

    info!("✔️ Successfully pulled and updated package");

    Ok(package_lineage)
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
    use crate::manifest::Row;

    #[test(tokio::test)]
    async fn test_no_pull_if_changes() -> Res {
        let storage = MockStorage::default();
        let lineage = PackageLineage::default();
        let mut manifest = Table::default();

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(PathBuf::from("foo"), Change::Added(Row::default()))]),
            ..InstalledPackageStatus::default()
        };
        let remote = MockRemote::default();
        let error = pull_package(
            lineage,
            &mut manifest,
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

    #[test(tokio::test)]
    async fn test_no_pull_if_diverged() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
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

    #[test(tokio::test)]
    async fn test_no_pull_if_up_to_date() {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
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
