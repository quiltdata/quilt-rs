use std::marker::Unpin;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::{debug, info, trace, warn};

use crate::Res;
use crate::flow;
use crate::installed_package::InstalledPackage;
use crate::io::manifest::RowsStream;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::DomainLineage;
use crate::lineage::Home;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::paths;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

/// This is the entrypoint for the lib.
/// All the work you can do with packages is done through calling `LocalDomain` methods.
#[derive(Debug)]
pub struct LocalDomain<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
    storage: S,
    /// Shared so a caller holding the domain behind a mutex can take a
    /// handle, release the mutex, and only then await a network round trip.
    /// See [`LocalDomain::remote_handle`].
    remote: Arc<R>,
}

impl LocalDomain {
    #[must_use]
    pub fn get_remote(&self) -> &RemoteS3 {
        &self.remote
    }

    /// A handle to *this* remote — the same client and credential caches,
    /// not a snapshot — that outlives a borrow of the domain.
    ///
    /// Callers that reach the domain through a mutex (the desktop app holds
    /// one around it) must use this for anything that awaits the network:
    /// `lock().await.get_remote().call().await` keeps the domain locked for
    /// the whole round trip, blocking every other command behind it.
    ///
    /// Deliberately not [`RemoteS3::try_clone`], which snapshots the client
    /// cache into a separate `RemoteS3`: a role switch clears the original's
    /// cache and the snapshot would keep signing as the old role.
    #[must_use]
    pub fn remote_handle(&self) -> Arc<RemoteS3> {
        Arc::clone(&self.remote)
    }

    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        let paths = paths::DomainPaths::new(root_dir.as_ref().to_path_buf());
        let lineage = lineage::DomainLineageIo::new(paths.lineage());
        let storage = LocalStorage::new();
        let remote = Arc::new(RemoteS3::new(paths.clone(), storage.clone()));
        Self {
            paths,
            lineage,
            storage,
            remote,
        }
    }

    pub async fn get_home(&self) -> Res<Home> {
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;
        Ok(lineage.home)
    }

    pub async fn set_home(&self, dir: impl AsRef<Path>) -> Res<Home> {
        info!("Setting home directory to {}", dir.as_ref().display());
        Ok(self.lineage.set_home(&self.storage, dir).await?.home)
    }

    pub async fn scaffold_paths_for_installing(&self, namespace: &Namespace) -> Res {
        debug!(
            "Scaffolding installation paths for namespace: {}",
            namespace
        );
        let home = self.get_home().await?;
        self.paths
            .scaffold_for_installing(&self.storage, &home, namespace)
            .await
    }

    pub async fn scaffold_paths_for_caching(&self, bucket: &str) -> Res {
        debug!("Scaffolding cache paths for bucket: {}", bucket);
        self.paths.scaffold_for_caching(&self.storage, bucket).await
    }

    pub async fn browse_remote_manifest(&self, uri: &ManifestUri) -> Res<Manifest> {
        self.scaffold_paths_for_caching(&uri.bucket).await?;

        debug!("Initiating browse flow for manifest {}", uri.hash);
        flow::browse(&self.paths, &self.storage, &*self.remote, uri)
            .await
            .inspect(|_| debug!("Successfully browsed manifest {}", uri.hash))
            .inspect_err(|e| warn!("Failed to browse manifest {}: {}", uri.hash, e))
    }

    pub fn create_installed_package(&self, namespace: Namespace) -> Res<InstalledPackage> {
        // TODO: seems like you can use PackageLineage as an argument instead of namespace
        Ok(InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace,
            paths: self.paths.clone(),
            remote: self.remote.try_clone()?,
            storage: self.storage.clone(),
        })
    }

    pub async fn install_package(&self, manifest_uri: &ManifestUri) -> Res<InstalledPackage> {
        info!("Installing package: {}", manifest_uri.namespace);
        debug!("Installing from manifest: {}", manifest_uri.display());

        debug!("Preparing paths for installation");
        self.scaffold_paths_for_caching(&manifest_uri.bucket)
            .await?;
        self.scaffold_paths_for_installing(&manifest_uri.namespace)
            .await?;

        debug!("Reading domain lineage");
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;

        debug!("Executing package installation flow");
        let lineage = flow::install_package(
            lineage,
            &self.paths,
            &self.storage,
            &*self.remote,
            manifest_uri,
        )
        .await?;

        debug!("Updating domain lineage");
        self.lineage.write(&self.storage, lineage).await?;

        info!("Successfully installed package: {}", manifest_uri.namespace);
        self.create_installed_package(manifest_uri.namespace.clone())
    }

    pub async fn create_package(
        &self,
        namespace: Namespace,
        source: Option<PathBuf>,
        message: Option<String>,
    ) -> Res<InstalledPackage> {
        info!("Creating package: {}", namespace);

        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;

        let lineage = flow::create(
            lineage,
            &self.paths,
            &self.storage,
            namespace.clone(),
            source,
            message,
        )
        .await?;

        self.lineage.write(&self.storage, lineage).await?;

        info!("Successfully created package: {}", namespace);
        self.create_installed_package(namespace)
    }

    pub async fn uninstall_package(&self, namespace: Namespace) -> Res<()> {
        info!("Uninstalling package: {}", namespace);

        debug!("Preparing paths for uninstallation");
        self.scaffold_paths_for_installing(&namespace).await?;

        debug!("Reading current lineage state");
        let lineage = self.lineage.read(&self.storage).await?;

        debug!("Executing package uninstallation flow");
        let lineage =
            flow::uninstall_package(lineage, &self.paths, &self.storage, namespace.clone()).await?;

        debug!("Updating domain lineage after uninstallation");
        self.lineage.write(&self.storage, lineage).await?;

        info!("Successfully uninstalled package: {}", namespace);
        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Res<Vec<InstalledPackage>> {
        // A pair of lines for a lookup that runs on every tick. The count is the
        // only part worth keeping, and only when it is surprising — so both go to
        // `trace` and the status line reports what was found.
        trace!("Listing installed packages");
        let lineage = self.lineage.read(&self.storage).await?;
        let namespaces = lineage.namespaces();

        trace!("Found {} installed packages", namespaces.len());
        let mut packages = Vec::with_capacity(namespaces.len());
        for namespace in namespaces {
            packages.push(self.create_installed_package(namespace)?);
        }
        Ok(packages)
    }

    /// The whole lineage record — every installed package, in one read.
    ///
    /// [`list_installed_packages`](Self::list_installed_packages) hands back
    /// an [`InstalledPackage`] per namespace, and each
    /// [`InstalledPackage::lineage`] call re-reads and re-parses this record
    /// to pluck one entry out of it. That is the right shape for operating on
    /// one package and the wrong one for surveying them all.
    pub async fn get_lineage(&self) -> Res<DomainLineage> {
        trace!("Reading domain lineage");
        self.lineage.read(&self.storage).await
    }

    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Res<Option<InstalledPackage>> {
        trace!("Looking up installed package: {}", namespace);
        let lineage = self.lineage.read(&self.storage).await?;
        if lineage.packages.contains_key(namespace) {
            trace!("Found installed package: {}", namespace);
            Ok(Some(self.create_installed_package(namespace.to_owned())?))
        } else {
            debug!("Package not found: {}", namespace);
            Ok(None)
        }
    }

    pub async fn build_manifest(
        &self,
        dest_path: PathBuf,
        stream: impl RowsStream + Unpin,
    ) -> Res<(PathBuf, String)> {
        info!("Building manifest at: {}", dest_path.display());
        let dest_dir = dest_path.parent().unwrap_or(&dest_path).to_path_buf();
        build_manifest_from_rows_stream(&self.storage, dest_dir, ManifestHeader::default(), stream)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;
    use test_log::test;

    use crate::lineage::PackageLineage;

    #[test(tokio::test)]
    async fn test_list_installed_packages() -> Res<()> {
        // Create a temporary directory for testing
        let temp_dir = TempDir::new()?;
        let local_domain = super::LocalDomain::new(temp_dir.path());

        // Set home directory
        local_domain.set_home(&temp_dir.path()).await?;

        // Initially there should be no packages
        let packages = local_domain.list_installed_packages().await?;
        assert!(packages.is_empty());

        // Add some packages to the lineage
        let mut lineage = local_domain.lineage.read(&local_domain.storage).await?;

        let namespaces = vec![
            Namespace::from(("foo", "bar")),
            Namespace::from(("test", "package")),
            Namespace::from(("abc", "xyz")),
        ];

        for namespace in &namespaces {
            lineage.packages.insert(
                namespace.clone(),
                PackageLineage {
                    commit: None,
                    remote_uri: Some(ManifestUri {
                        bucket: "test-bucket".to_string(),
                        namespace: namespace.clone(),
                        hash: "abcdef".to_string(),
                        origin: None,
                    }),
                    base_hash: "abcdef".to_string(),
                    latest_hash: "abcdef".to_string(),
                    paths: std::collections::BTreeMap::new(),
                    ..PackageLineage::default()
                },
            );
        }

        local_domain
            .lineage
            .write(&local_domain.storage, lineage)
            .await?;

        // Now list_installed_packages should return packages in sorted order
        let packages = local_domain.list_installed_packages().await?;
        assert_eq!(packages.len(), 3);

        // Check that packages are returned in sorted order by namespace
        assert_eq!(packages[0].namespace, Namespace::from(("abc", "xyz")));
        assert_eq!(packages[1].namespace, Namespace::from(("foo", "bar")));
        assert_eq!(packages[2].namespace, Namespace::from(("test", "package")));

        Ok(())
    }
}
