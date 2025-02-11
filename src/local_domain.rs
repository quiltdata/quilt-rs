use std::marker::Unpin;
use std::path::Path;
use std::path::PathBuf;

use serde_yaml::Mapping;
use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

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
use crate::manifest::JsonObject;
use crate::manifest::Table;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

/// This is the entrypoint for the lib.
/// All the work you can do with packages is done through calling `LocalDomain` methods.
#[derive(Debug)]
pub struct LocalDomain<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
    storage: S,
    remote: R,
}

impl LocalDomain {
    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        let paths = paths::DomainPaths::new(root_dir.as_ref().to_path_buf());
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


    pub async fn browse_remote_manifest(&self, uri: &ManifestUri) -> Res<Table> {
        flow::browse(&self.paths, &self.storage, &self.remote, uri).await
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

        self.create_installed_package(manifest_uri.namespace.clone())
    }

    pub async fn uninstall_package(&self, namespace: Namespace) -> Res<()> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage =
            flow::uninstall_package(lineage, &self.paths, &self.storage, namespace).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Res<Vec<InstalledPackage>> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut namespaces: Vec<Namespace> = lineage.packages.into_keys().collect();
        namespaces.sort();
        let mut packages = Vec::new();
        for namespace in namespaces {
            packages.push(self.create_installed_package(namespace)?);
        }
        Ok(packages)
    }

    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Res<Option<InstalledPackage>> {
        let lineage = self.lineage.read(&self.storage).await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(self.create_installed_package(namespace.to_owned())?))
        } else {
            Ok(None)
        }
    }

    pub async fn package_s3_prefix(
        &self,
        source_uri: &S3Uri,
        dest_uri: S3PackageUri,
        message: Option<String>,
        user_meta: Option<JsonObject>,
    ) -> Res<ManifestUri> {
        flow::package_s3_prefix(
            &self.paths,
            &self.storage,
            &self.remote,
            source_uri,
            dest_uri,
            message,
            user_meta,
        )
        .await
    }

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
