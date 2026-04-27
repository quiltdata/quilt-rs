use tracing::debug;
use tracing::info;

use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::paths;
use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use quilt_uri::Namespace;

/// Uninstall package: remove files from working directory, manifest from `.quilt` and from
/// `.quilt/lineage.json`.
pub async fn uninstall_package(
    mut lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &impl Storage,
    namespace: Namespace,
) -> Res<DomainLineage> {
    info!("⏳ Uninstalling package {}", namespace);

    debug!("🔍 Checking if package exists in lineage");
    lineage
        .packages
        .remove(&namespace)
        .ok_or(Error::InstallPackage(InstallPackageError::NotInstalled(
            namespace.to_owned(),
        )))?;
    debug!("✔️ Package removed from lineage");

    debug!("⏳ Removing installed manifests");
    let manifest_path = paths.installed_manifests_dir(&namespace);
    storage.remove_dir_all(&manifest_path).await?;
    debug!("✔️ Removed manifests at: {}", manifest_path.display());

    debug!("⏳ Removing working directory");
    let package_home = paths::package_home(&lineage.home, &namespace);
    storage.remove_dir_all(&package_home).await?;
    debug!("✔️ Removed working directory: {}", package_home.display());

    // TODO: Remove object files? But need to make sure no other manifest uses them.
    debug!("ℹ️ Skipping object files cleanup - may be used by other packages");

    info!("✔️ Successfully uninstalled package {}", namespace);
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use test_log::test;

    use super::*;

    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Home;
    use crate::lineage::PackageLineage;

    #[test(tokio::test)]
    async fn test_panic_if_no_installed_package() {
        let lineage = DomainLineage::default();
        let storage = MockStorage::default();
        let paths = paths::DomainPaths::default();

        let namespace: Namespace = ("foo", "bar").into();
        let result = uninstall_package(lineage, &paths, &storage, namespace.clone()).await;
        assert!(matches!(
            result.unwrap_err(),
            Error::InstallPackage(InstallPackageError::NotInstalled(ns)) if ns == namespace
        ));
    }

    #[test(tokio::test)]
    async fn test_uninstall_package() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));

        let paths = paths::DomainPaths::default();
        let storage = MockStorage::default();

        paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        let lineage = DomainLineage {
            home,
            packages: BTreeMap::from([(namespace.clone(), PackageLineage::default())]),
        };

        let lineage = uninstall_package(lineage, &paths, &storage, namespace).await?;
        assert!(lineage.packages.is_empty());
        Ok(())
    }
}
