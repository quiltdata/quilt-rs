use crate::paths;
use crate::quilt::flow::browse::cache_remote_manifest;
use crate::quilt::lineage::DomainLineage;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::manifest_handle::RemoteManifest;
use crate::quilt::Storage;
use crate::s3_utils;
use crate::Error;

pub async fn install_package(
    lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &mut impl Storage,
    remote_manifest: &RemoteManifest,
) -> Result<DomainLineage, Error> {
    let remote = s3_utils::RemoteS3::new();
    // bail if already installed
    // TODO: if compatible (same remote), just return the installed package
    if lineage.packages.contains_key(&remote_manifest.namespace) {
        return Err(Error::PackageAlreadyInstalled(
            remote_manifest.namespace.clone(),
        ));
    }

    cache_remote_manifest(paths, storage, &remote, remote_manifest).await?;

    // Make an "installed" copy of the remote manifest.
    let installed_manifest_path =
        paths.installed_manifest(&remote_manifest.namespace, &remote_manifest.hash);
    storage
        .create_dir_all(&installed_manifest_path.parent().unwrap())
        .await?;
    paths::copy_cached_to_installed(
        paths,
        &remote_manifest.bucket,
        &remote_manifest.namespace,
        &remote_manifest.hash,
    )
    .await?;

    // Create the identity cache dir.
    let objects_dir = paths.objects_dir();
    storage.create_dir_all(&objects_dir).await?;

    // Create the working dir.
    let working_dir = paths.working_dir(&remote_manifest.namespace);
    storage.create_dir_all(&working_dir).await?;

    // Resolve and record latest manifest hash
    let latest_hash = remote_manifest.resolve_latest().await?;
    // Update the lineage (with empty paths).
    let mut lineage = lineage;
    lineage.packages.insert(
        remote_manifest.namespace.clone(),
        PackageLineage::from_remote(remote_manifest.to_owned(), latest_hash),
    );
    Ok(lineage)
}
