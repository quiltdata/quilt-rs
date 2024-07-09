use std::marker::Unpin;
use std::path::PathBuf;

use crate::flow;
use crate::installed_package::InstalledPackage;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::DomainLineage;
use crate::manifest::Header;
use crate::manifest::Table;
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Res;

/// This is the entrypoint for the lib.
/// All the work you can do with packages is done through calling `LocalDomain` methods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDomain<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
    storage: S,
    remote: R,
}

impl LocalDomain {
    /// Creates new `LocalDomain` instance.
    /// Everything will be stored in `root_dir`: .quilt directory and working directories for each package.
    /// Storage and Remote are set to default implementations: `LocalStorage` and `RemoteS3`.
    pub fn new(root_dir: PathBuf) -> Self {
        let paths = paths::DomainPaths::new(root_dir.clone());
        let lineage = lineage::DomainLineageIo::new(paths.lineage());
        let storage = LocalStorage::new();
        let remote = RemoteS3::new();
        Self {
            lineage,
            paths,
            remote,
            storage,
        }
    }

    /// Calls the `flow::browse` with LocalDomain's propreties.
    /// Note that when we "browse" remote manifest, we always cache it in .quilt directory.
    pub async fn browse_remote_manifest(&self, uri: &ManifestUri) -> Res<Table> {
        flow::browse(&self.paths, &self.storage, &self.remote, uri).await
    }

    // TODO: make public only for tests
    /// It is public only for tests in QuiltSync. Please, try to not use it
    pub fn create_installed_package(&self, namespace: Namespace) -> InstalledPackage {
        // TODO: seems like you can use PackageLineage as an argument instead of namespace
        InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace: namespace.clone(),
            paths: self.paths.clone(),
            remote: self.remote.clone(),
            storage: self.storage.clone(),
        }
    }

    /// Calls the `flow::install_package` and writes the package data to lineage
    pub async fn install_package(&self, manifest_uri: &ManifestUri) -> Res<InstalledPackage> {
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;
        let lineage = flow::install_package(
            lineage,
            &self.paths,
            &self.storage,
            &self.remote,
            manifest_uri,
        )
        .await?;
        self.lineage.write(&self.storage, lineage).await?;

        Ok(self.create_installed_package(manifest_uri.namespace.clone()))
    }

    /// Calls the `flow::uninstall_package` and removes the package data from lineage
    pub async fn uninstall_package(&self, namespace: Namespace) -> Res<()> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage =
            flow::uninstall_package(lineage, &self.paths, &self.storage, namespace).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    /// Gets list of namespaces from lineage, creates `InstalledPackage` for each namespace and returns them.
    pub async fn list_installed_packages(&self) -> Res<Vec<InstalledPackage>> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut namespaces: Vec<Namespace> = lineage.packages.into_keys().collect();
        namespaces.sort();
        let packages = namespaces
            .into_iter()
            .map(|namespace| self.create_installed_package(namespace))
            .collect();
        Ok(packages)
    }

    /// Returns `InstalledPackage`or `None` if lineage doesn't contain that package.
    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Res<Option<InstalledPackage>> {
        let lineage = self.lineage.read(&self.storage).await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(self.create_installed_package(namespace.to_owned())))
        } else {
            Ok(None)
        }
    }

    /// Calls `flow::package_s3_prefix` with LocalDomain's properties.
    pub async fn package_s3_prefix(
        &self,
        source_uri: &S3Uri,
        dest_uri: S3PackageUri,
    ) -> Res<ManifestUri> {
        flow::package_s3_prefix(
            &self.paths,
            &self.storage,
            &self.remote,
            source_uri,
            dest_uri,
        )
        .await
    }

    /// Builds manifest from rows stream and writes it file in `dest_path`
    pub async fn build_manifest(
        &self,
        dest_path: PathBuf,
        stream: impl RowsStream + Unpin,
    ) -> Res<(PathBuf, String)> {
        let manifest_path = |_t: &str| dest_path.clone();
        build_manifest_from_rows_stream(&self.storage, manifest_path, Header::default(), stream)
            .await
    }
}
