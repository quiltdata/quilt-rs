use std::path::PathBuf;

use crate::{
    paths::{copy_cached_to_installed, DomainPaths},
    Error,
};

use crate::quilt::{
    flow::browse::cache_remote_manifest, flow::install_paths::install_paths,
    flow::uninstall_paths::uninstall_paths, lineage::PackageLineage,
    manifest_handle::ReadableManifest,
};

pub async fn reset_to_latest(
    lineage: PackageLineage,
    manifest: &(impl ReadableManifest + Sync),
    paths: &DomainPaths,
    working_dir: PathBuf,
    namespace: String,
) -> Result<PackageLineage, Error> {
    let new_latest = lineage.remote.resolve_latest().await?;
    if new_latest == lineage.remote.hash {
        // already at latest
        return Ok(lineage);
    }

    let entries_paths: Vec<String> = lineage.paths.clone().into_keys().collect();
    let mut lineage = uninstall_paths(lineage, working_dir.clone(), &entries_paths).await?;

    lineage.latest_hash = new_latest.clone();
    lineage.remote.hash = new_latest.clone();
    lineage.base_hash = new_latest;

    cache_remote_manifest(paths, &lineage.remote).await?;
    copy_cached_to_installed(
        paths,
        &lineage.remote.bucket,
        &namespace.to_string(),
        &lineage.remote.hash,
    )
    .await?;

    let materialized_manifest = manifest.read().await?;
    let paths_to_install = entries_paths
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
