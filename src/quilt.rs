use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

use crate::flow::browse::browse_remote_manifest;
use crate::flow::certify_latest::certify_latest;
use crate::flow::commit::commit_package;
use crate::flow::install_package::install_package;
use crate::flow::install_paths::install_paths;
use crate::flow::package::package_s3_prefix;
use crate::flow::pull::pull_package;
use crate::flow::push::push_package;
use crate::flow::reset_to_latest::reset_to_latest;
use crate::flow::status::create_status;
use crate::flow::status::refresh_latest_hash;
use crate::flow::uninstall_package::uninstall_package;
use crate::flow::uninstall_paths::uninstall_paths;
use crate::io::remote::RemoteS3;
use crate::io::remote::Remote;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::CommitState;
use crate::lineage::DomainLineage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineagePaths;
use crate::manifest::JsonObject;
use crate::manifest::Table;
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;

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

    pub async fn browse_remote_manifest(&self, uri: &ManifestUri) -> Result<Table, Error> {
        browse_remote_manifest(&self.paths, &self.storage, &self.remote, uri).await
    }

    fn create_installed_package(&self, namespace: Namespace) -> InstalledPackage {
        // TODO: seems like you can use PackageLineage as an argument instead of namespace
        InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace: namespace.clone(),
            paths: self.paths.clone(),
            remote: self.remote.clone(),
            storage: self.storage.clone(),
        }
    }

    pub async fn install_package(
        &self,
        manifest_uri: &ManifestUri,
    ) -> Result<InstalledPackage, Error> {
        // Read the lineage
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;
        let lineage = install_package(
            lineage,
            &self.paths,
            &self.storage,
            &self.remote,
            manifest_uri,
        )
        .await?;
        let _fixme = self.lineage.write(&self.storage, lineage).await?;

        Ok(self.create_installed_package(manifest_uri.namespace.clone()))
    }

    pub async fn uninstall_package(&self, namespace: Namespace) -> Result<(), Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage = uninstall_package(lineage, &self.paths, &self.storage, namespace).await?;
        let _fixme = self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Result<Vec<InstalledPackage>, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut namespaces: Vec<Namespace> = lineage.packages.into_keys().collect();
        namespaces.sort();
        let packages = namespaces
            .into_iter()
            .map(|namespace| self.create_installed_package(namespace))
            .collect();
        Ok(packages)
    }

    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Result<Option<InstalledPackage>, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(self.create_installed_package(namespace.to_owned())))
        } else {
            Ok(None)
        }
    }

    pub async fn package_s3_prefix(
        &self,
        source_uri: &S3Uri,
        dest_uri: S3PackageUri,
    ) -> Result<ManifestUri, Error> {
        package_s3_prefix(
            &self.paths,
            &self.storage,
            &self.remote,
            source_uri,
            dest_uri,
        )
        .await
    }
}

/// Similar to `LocalDomain` because it has access to the same lineage file and remote/storage
/// traits.
/// But it only manages one particular installed package.
/// It can be instantiated from `LocalDomain` by installing new or listing existing packages.
#[derive(Clone, Debug, PartialEq)]
pub struct InstalledPackage<S: Storage + Clone = LocalStorage, R: Remote + Clone = RemoteS3> {
    lineage: lineage::PackageLineageIo,
    paths: paths::DomainPaths,
    remote: R,
    storage: S,
    pub namespace: Namespace,
}

impl InstalledPackage {
    pub async fn manifest(&self) -> Result<Table, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let pathbuf = self
            .paths
            .installed_manifest(&self.namespace, lineage.current_hash());
        Table::read_from_path(&self.storage, &pathbuf).await
    }

    pub async fn lineage(&self) -> Result<lineage::PackageLineage, Error> {
        self.lineage.read(&self.storage).await
    }

    pub fn working_folder(&self) -> PathBuf {
        self.paths.working_dir(&self.namespace)
    }

    pub async fn status(&self) -> Result<InstalledPackageStatus, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage = refresh_latest_hash(lineage, &self.remote).await?;
        let manifest = self.manifest().await?;
        let (lineage, status) =
            create_status(lineage, &self.storage, &manifest, self.working_folder()).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &Vec<PathBuf>) -> Result<LineagePaths, Error> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = install_paths(
            lineage,
            &mut manifest,
            &self.paths,
            self.working_folder(),
            self.namespace.clone(),
            &self.storage,
            &self.remote,
            paths,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn uninstall_paths(&self, paths: &Vec<PathBuf>) -> Result<LineagePaths, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage = uninstall_paths(lineage, self.working_folder(), &self.storage, paths).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        log::debug!("revert_paths: {paths:?}");
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<JsonObject>,
    ) -> Result<Option<CommitState>, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;

        let (lineage, status) =
            create_status(lineage, &self.storage, &manifest, self.working_folder()).await?;

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            self.working_folder(),
            status,
            self.namespace.clone(),
            message,
            user_meta,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.commit)
    }

    pub async fn push(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let manifest = self.manifest().await?;
        let lineage = push_package(
            lineage,
            manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            Some(self.namespace.clone()),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn pull(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let (lineage, status) =
            create_status(lineage, &self.storage, &manifest, self.working_folder()).await?;
        let lineage = pull_package(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            self.working_folder(),
            status,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn certify_latest(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let latest_manifest_uri = lineage.remote.clone();
        let lineage = certify_latest(lineage, &self.remote, latest_manifest_uri).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn reset_to_latest(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = reset_to_latest(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            self.working_folder(),
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }
}
