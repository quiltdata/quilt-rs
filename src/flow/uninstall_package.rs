use tracing::{debug, info};

use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::paths;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

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
        .ok_or(Error::PackageNotInstalled(namespace.to_owned()))?;
    debug!("✔️ Package removed from lineage");

    debug!("⏳ Removing installed manifests");
    let manifest_path = paths.installed_manifests(&namespace);
    storage.remove_dir_all(&manifest_path).await?;
    debug!("✔️ Removed manifests at: {}", manifest_path.display());

    debug!("⏳ Removing working directory");
    let working_dir = paths.working_dir(&namespace);
    storage.remove_dir_all(&working_dir).await?;
    debug!("✔️ Removed working directory: {}", working_dir.display());

    // TODO: Remove object files? But need to make sure no other manifest uses them.
    debug!("ℹ️ Skipping object files cleanup - may be used by other packages");

    info!("✔️ Successfully uninstalled package {}", namespace);
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    use crate::lineage::PackageLineage;

    use crate::io::storage::mocks::MockStorage;
    use crate::paths::scaffold_paths;

    #[tokio::test]
    async fn test_panic_if_no_installed_package() {
        let lineage = DomainLineage::default();
        let storage = MockStorage::default();
        let paths = paths::DomainPaths::default();

        let result = uninstall_package(lineage, &paths, &storage, ("foo", "bar").into()).await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "The given package is not installed: foo/bar"
        )
    }

    #[tokio::test]
    async fn test_uninstall_package() -> Res {
        let lineage = DomainLineage {
            packages: BTreeMap::from([(("foo", "bar").into(), PackageLineage::default())]),
        };
        let paths = paths::DomainPaths::default();
        let storage = MockStorage::default();

        let namespace = Namespace::from(("foo", "bar"));

        scaffold_paths(&storage, paths.required_installed_package_paths(&namespace)).await?;

        let lineage = uninstall_package(lineage, &paths, &storage, namespace).await?;
        assert!(lineage.packages.is_empty());
        Ok(())
    }
}
