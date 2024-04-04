use crate::paths;
use crate::quilt::lineage::DomainLineage;
use crate::Error;
use tokio::fs::remove_dir_all;
use tracing::log;

pub async fn uninstall_package(
    mut lineage: DomainLineage,
    paths: &paths::DomainPaths,
    namespace: impl AsRef<str>,
) -> Result<(), Error> {
    let namespace = namespace.as_ref();

    lineage
        .packages
        .remove(namespace)
        .ok_or(Error::PackageNotInstalled(namespace.to_owned()))?;

    if let Err(err) = remove_dir_all(paths.installed_manifests(namespace)).await {
        log::error!("Failed to remove installed manifests: {err}");
    }
    if let Err(err) = remove_dir_all(paths.working_dir(namespace)).await {
        log::error!("Failed to remove working directory: {err}");
    }

    // TODO: Remove object files? But need to make sure no other manifest uses them.

    Ok(())
}
