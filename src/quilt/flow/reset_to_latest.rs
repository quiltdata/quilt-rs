use std::path::PathBuf;

use crate::{
    paths::{copy_cached_to_installed, DomainPaths},
    Error,
};

use crate::quilt::{
    flow::browse::cache_remote_manifest, flow::install_paths::install_paths,
    flow::uninstall_paths::uninstall_paths, lineage::PackageLineageIo,
    manifest_handle::ReadableManifest,
};

pub async fn reset_to_latest(
    lineage_io: &PackageLineageIo,
    manifest: &(impl ReadableManifest + Sync),
    paths: &DomainPaths,
    working_dir: PathBuf,
    namespace: String,
) -> Result<(), Error> {
    let lineage = lineage_io.read().await?;

    let new_latest = lineage.remote.resolve_latest().await?;
    if new_latest == lineage.remote.hash {
        // already at latest
        return Ok(());
    }

    let entries_paths: Vec<String> = lineage.paths.into_keys().collect();
    uninstall_paths(lineage_io, working_dir.clone(), &entries_paths).await?;
    let mut lineage = lineage_io.read().await?;

    lineage.latest_hash = new_latest.clone();
    lineage.remote.hash = new_latest.clone();
    lineage.base_hash = new_latest;

    cache_remote_manifest(&paths, &lineage.remote).await?;
    copy_cached_to_installed(
        &paths,
        &lineage.remote.bucket,
        &namespace.to_string(),
        &lineage.remote.hash,
    )
    .await?;

    lineage_io.write(lineage).await?;

    let materialized_manifest = manifest.read().await?;
    let paths_to_install = entries_paths
        .into_iter()
        .filter(|x| materialized_manifest.records.contains_key(x))
        .collect();
    install_paths(
        lineage_io,
        manifest,
        paths,
        working_dir,
        namespace,
        &paths_to_install,
    )
    .await
}
