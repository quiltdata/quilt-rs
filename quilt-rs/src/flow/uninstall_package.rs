use tracing::debug;
use tracing::info;

use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use crate::io::storage::Storage;
use crate::lineage::Home;
use crate::paths;
use quilt_uri::Namespace;

/// Uninstall package: remove files from working directory and manifests from
/// `.quilt`. `installed` says whether the domain has the package; the caller
/// removes its lineage.
pub async fn uninstall_package(
    home: &Home,
    installed: bool,
    paths: &paths::DomainPaths,
    storage: &impl Storage,
    namespace: Namespace,
) -> Res {
    info!("⏳ Uninstalling package {}", namespace);

    if !installed {
        return Err(Error::InstallPackage(InstallPackageError::NotInstalled(
            namespace,
        )));
    }

    debug!("⏳ Removing installed manifests");
    let manifest_path = paths.installed_manifests_dir(&namespace);
    storage.remove_dir_all(&manifest_path).await?;
    debug!("✔️ Removed manifests at: {}", manifest_path.display());

    debug!("⏳ Removing working directory");
    let package_home = paths::package_home(home, &namespace);
    storage.remove_dir_all(&package_home).await?;
    debug!("✔️ Removed working directory: {}", package_home.display());

    // Objects may be used by other packages. A pruning uninstall deletes the
    // ones no other manifest uses afterwards; see `flow::gc::prune`.

    info!("✔️ Successfully uninstalled package {}", namespace);
    Ok(())
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    use crate::io::storage::mocks::MockStorage;

    #[test(tokio::test)]
    async fn test_panic_if_no_installed_package() {
        let storage = MockStorage::default();
        let paths = paths::DomainPaths::default();

        let namespace: Namespace = ("foo", "bar").into();
        let result =
            uninstall_package(&Home::default(), false, &paths, &storage, namespace.clone()).await;
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

        uninstall_package(&home, true, &paths, &storage, namespace.clone()).await?;
        assert!(!storage.exists(paths::package_home(&home, &namespace)).await);
        assert!(
            !storage
                .exists(paths.installed_manifests_dir(&namespace))
                .await
        );
        Ok(())
    }
}
