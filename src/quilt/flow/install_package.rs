use crate::paths;
use crate::quilt::flow::browse::cache_remote_manifest;
use crate::quilt::lineage::DomainLineage;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::manifest_handle::RemoteManifest;
use crate::quilt::Storage;
use crate::Error;

pub async fn install_package(
    lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &mut impl Storage,
    remote: &RemoteManifest,
) -> Result<DomainLineage, Error> {
    // bail if already installed
    // TODO: if compatible (same remote), just return the installed package
    if lineage.packages.contains_key(&remote.namespace) {
        return Err(Error::PackageAlreadyInstalled(remote.namespace.clone()));
    }

    cache_remote_manifest(paths, storage, remote).await?;

    // Make an "installed" copy of the remote manifest.
    let installed_manifest_path = paths.installed_manifest(&remote.namespace, &remote.hash);
    storage
        .create_dir_all(&installed_manifest_path.parent().unwrap())
        .await?;
    paths::copy_cached_to_installed(paths, &remote.bucket, &remote.namespace, &remote.hash).await?;

    // Create the identity cache dir.
    let objects_dir = paths.objects_dir();
    storage.create_dir_all(&objects_dir).await?;

    // Create the working dir.
    let working_dir = paths.working_dir(&remote.namespace);
    storage.create_dir_all(&working_dir).await?;

    // Resolve and record latest manifest hash
    let latest_hash = remote.resolve_latest().await?;
    // Update the lineage (with empty paths).
    let mut lineage = lineage;
    lineage.packages.insert(
        remote.namespace.clone(),
        PackageLineage::from_remote(remote.to_owned(), latest_hash),
    );
    Ok(lineage)
}
