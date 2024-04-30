use tracing::log;

use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::paths;
use crate::quilt::uri::Namespace;
use crate::Error;

pub async fn uninstall_package(
    mut lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &impl Storage,
    namespace: Namespace,
) -> Result<DomainLineage, Error> {
    log::debug!("Uninstalling package {}", namespace);

    lineage
        .packages
        .remove(&namespace)
        .ok_or(Error::PackageNotInstalled(namespace.to_owned()))?;

    storage
        .remove_dir_all(paths.installed_manifests(&namespace))
        .await?;
    storage
        .remove_dir_all(paths.working_dir(&namespace))
        .await?;

    // TODO: Remove object files? But need to make sure no other manifest uses them.

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    use crate::lineage::PackageLineage;
    use crate::quilt::mocks;

    #[tokio::test]
    async fn test_panic_if_no_installed_package() {
        let lineage = DomainLineage::default();
        let storage = mocks::storage::MockStorage::default();
        let paths = paths::DomainPaths::default();

        let result = uninstall_package(lineage, &paths, &storage, ("foo", "bar").into()).await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "The given package is not installed: foo/bar"
        )
    }

    #[tokio::test]
    async fn test_uninstall_package() -> Result<(), Error> {
        let lineage = DomainLineage {
            packages: BTreeMap::from([(("foo", "bar").into(), PackageLineage::default())]),
        };
        let paths = paths::DomainPaths::default();
        let storage = mocks::storage::MockStorage::default();
        let namespace = Namespace::from(("foo", "bar"));
        storage
            .create_dir_all(paths.installed_manifests(&namespace))
            .await?;
        storage
            .create_dir_all(paths.working_dir(&namespace))
            .await?;

        let lineage = uninstall_package(lineage, &paths, &storage, namespace).await?;
        assert!(lineage.packages.is_empty());
        Ok(())
    }
}
