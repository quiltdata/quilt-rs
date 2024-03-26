use std::path::PathBuf;

use crate::paths::{copy_cached_to_installed, DomainPaths};
use crate::quilt::flow::browse::cache_remote_manifest;
use crate::quilt::flow::install_paths::install_paths;
use crate::quilt::flow::status::create_status;
use crate::quilt::flow::uninstall_paths::uninstall_paths;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::manifest_handle;
use crate::quilt::Error;

pub async fn pull_package(
    lineage: PackageLineage,
    manifest: &(impl manifest_handle::ReadableManifest + Sync),
    paths: &DomainPaths,
    working_dir: PathBuf,
    namespace: String,
) -> Result<PackageLineage, Error> {
    let (lineage, status) = create_status(lineage, manifest, working_dir.clone()).await?;
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
    let mut lineage = uninstall_paths(lineage, working_dir.clone(), &installed_paths).await?;

    // TODO: uninstall_paths() just modified the lineage, so re-reading it here.
    // There needs to be a better way.
    lineage.remote.hash = lineage.latest_hash.clone();
    lineage.base_hash = lineage.latest_hash.clone();

    cache_remote_manifest(paths, &lineage.remote).await?;
    copy_cached_to_installed(
        paths,
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
        &paths_to_install,
    )
    .await
}
