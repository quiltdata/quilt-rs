use tracing::log;

use crate::paths;
use crate::quilt::lineage::DomainLineage;
use crate::quilt::Storage;
use crate::Error;

pub async fn uninstall_package(
    mut lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &impl Storage,
    namespace: impl AsRef<str>,
) -> Result<DomainLineage, Error> {
    let namespace = namespace.as_ref();
    log::debug!("Uninstalling package {}", namespace);

    lineage
        .packages
        .remove(namespace)
        .ok_or(Error::PackageNotInstalled(namespace.to_owned()))?;

    storage
        .remove_dir_all(paths.installed_manifests(namespace))
        .await?;
    storage.remove_dir_all(paths.working_dir(namespace)).await?;

    // TODO: Remove object files? But need to make sure no other manifest uses them.

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    use crate::quilt::lineage::PackageLineage;
    use crate::quilt::storage::mock_storage::MockStorage;

    #[tokio::test]
    async fn test_panic_if_no_installed_package() {
        let lineage = DomainLineage::default();
        let storage = MockStorage::default();
        let paths = paths::DomainPaths::default();

        let result = uninstall_package(lineage, &paths, &storage, "foo/bar").await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "The given package is not installed: foo/bar"
        )
    }

    #[tokio::test]
    async fn test_uninstall_package() -> Result<(), Error> {
        let lineage = DomainLineage {
            packages: BTreeMap::from([("foo/bar".to_string(), PackageLineage::default())]),
        };
        let storage = MockStorage::default();
        let paths = paths::DomainPaths::default();

        let lineage = uninstall_package(lineage, &paths, &storage, "foo/bar").await?;
        assert!(lineage.packages.is_empty());
        Ok(())
    }
}
