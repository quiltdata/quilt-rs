use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mockall::automock;
use mockall::predicate::str;

use tokio::sync;

use tokio_stream::StreamExt;

use crate::error::Error;
use crate::quilt;
use crate::telemetry::prelude::*;

use quilt_rs::flow::UserMeta;
use quilt_rs::io::remote::HostConfig;

/// Result of checking whether a package is already installed.
#[derive(Debug)]
pub enum InstallCheck {
    /// Exact same hash — already up to date
    AlreadyInstalled,
    /// Same namespace, different hash — needs pull.
    /// Contains the hash of the currently installed version.
    DifferentVersion(String),
    /// Installed locally without a remote origin
    LocalOnly,
    /// Not installed at all
    NotInstalled,
}

/// Result of attempting to install a package.
#[derive(Debug, PartialEq)]
pub enum InstallOutcome {
    /// Package was installed (or was already installed with the same hash).
    Installed,
    /// A different version is already installed.
    DifferentVersion {
        requested_hash: String,
        installed_hash: String,
    },
    /// Installed locally without a remote origin.
    LocalOnly,
}

pub struct Model {
    quilt: sync::Mutex<quilt::LocalDomain>,
}

#[automock]
pub trait QuiltModel {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain>;

    async fn browse_remote_manifest(
        &self,
        remote_manifest: &quilt_uri::ManifestUri,
    ) -> Result<quilt::manifest::Manifest, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .browse_remote_manifest(remote_manifest)
            .await?)
    }

    async fn get_installed_packages_list(&self) -> Result<Vec<quilt::InstalledPackage>, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .list_installed_packages()
            .await?)
    }

    async fn get_installed_package(
        &self,
        namespace: &quilt_uri::Namespace,
    ) -> Result<Option<quilt::InstalledPackage>, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .get_installed_package(namespace)
            .await?)
    }

    async fn get_installed_package_lineage(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt::lineage::PackageLineage, Error> {
        Ok(package.lineage().await?)
    }

    async fn get_installed_package_records(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<BTreeMap<PathBuf, quilt::manifest::ManifestRow>, Error> {
        let manifest = package.manifest().await?;
        let mut stream = manifest.records_stream().await;
        let mut records = BTreeMap::new();
        while let Some(page) = stream.next().await {
            if let Ok(rows) = page {
                for row in rows.into_iter().flatten() {
                    records.insert(row.logical_key.clone(), row);
                }
            }
        }
        Ok(records)
    }

    async fn get_installed_package_status(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::lineage::InstalledPackageStatus, Error> {
        Ok(package.status(host_config).await?)
    }

    async fn recompute_local_status(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::lineage::InstalledPackageStatus, Error> {
        Ok(package.recompute_local_status(host_config).await?)
    }

    async fn package_commit(
        &self,
        package: &quilt::InstalledPackage,
        message: String,
        metadata: UserMeta,
        workflow: Option<quilt::manifest::Workflow>,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::lineage::CommitState, Error> {
        Ok(package
            .commit(message, metadata, workflow, host_config)
            .await?)
    }

    async fn package_install_paths(
        &self,
        package: &quilt::InstalledPackage,
        paths: &[PathBuf],
    ) -> Result<quilt::lineage::LineagePaths, Error> {
        Ok(package.install_paths(paths).await?)
    }

    async fn package_pull(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(package.pull(host_config).await?)
    }

    async fn is_package_installed(
        &self,
        manifest_uri: &quilt_uri::ManifestUri,
    ) -> Result<InstallCheck, Error> {
        match self.get_installed_package(&manifest_uri.namespace).await? {
            Some(installed_package) => {
                let package_lineage = self
                    .get_installed_package_lineage(&installed_package)
                    .await?;
                let Some(installed_manifest_uri) = package_lineage.remote_uri.as_ref() else {
                    return Ok(InstallCheck::LocalOnly);
                };
                if manifest_uri.hash == installed_manifest_uri.hash {
                    Ok(InstallCheck::AlreadyInstalled)
                } else {
                    Ok(InstallCheck::DifferentVersion(
                        installed_manifest_uri.hash.clone(),
                    ))
                }
            }
            None => Ok(InstallCheck::NotInstalled),
        }
    }

    async fn is_path_installed(
        &self,
        package: &quilt::InstalledPackage,
        path: &PathBuf,
    ) -> Result<bool, Error> {
        let package_lineage = self.get_installed_package_lineage(package).await?;
        Ok(package_lineage.paths.contains_key(path))
    }

    async fn package_push(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::PushOutcome, Error> {
        Ok(package.push(host_config).await?)
    }

    async fn package_publish(
        &self,
        package: &quilt::InstalledPackage,
        message: String,
        metadata: UserMeta,
        workflow: Option<quilt::manifest::Workflow>,
        host_config: Option<HostConfig>,
        status: Option<quilt::lineage::InstalledPackageStatus>,
    ) -> Result<quilt::PublishOutcome, Error> {
        Ok(package
            .publish(message, metadata, workflow, host_config, status)
            .await?)
    }

    /// Resolve a workflow id (`Option<String>`) into the materialised
    /// [`quilt::manifest::Workflow`] the remote enforces. Wrapping the
    /// `InstalledPackage` method on the trait lets the autosync tick
    /// path go through `model::package_publish` (free function) without
    /// hitting real storage in mock-based unit tests.
    async fn resolve_workflow(
        &self,
        package: &quilt::InstalledPackage,
        workflow: Option<String>,
    ) -> Result<Option<quilt::manifest::Workflow>, Error> {
        Ok(package.resolve_workflow(workflow).await?)
    }

    async fn package_revision_certify_latest(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(package.certify_latest().await?)
    }

    async fn package_revision_reset_local(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(package.reset_to_latest().await?)
    }

    async fn set_remote(
        &self,
        package: &quilt::InstalledPackage,
        origin: quilt_uri::Host,
        bucket: String,
    ) -> Result<(), Error> {
        Ok(package.set_remote(bucket, Some(origin)).await?)
    }

    async fn package_create(
        &self,
        namespace: quilt_uri::Namespace,
        source: Option<PathBuf>,
        message: Option<String>,
    ) -> Result<quilt::InstalledPackage, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .create_package(namespace, source, message)
            .await?)
    }

    async fn package_install(
        &self,
        remote_manifest: &quilt_uri::ManifestUri,
    ) -> Result<quilt::InstalledPackage, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .install_package(remote_manifest)
            .await?)
    }

    async fn package_uninstall(&self, namespace: quilt_uri::Namespace) -> Result<(), Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .uninstall_package(namespace)
            .await?)
    }

    async fn package_home(&self, namespace: &quilt_uri::Namespace) -> Result<PathBuf, Error> {
        let installed_package = self
            .get_installed_package(namespace)
            .await?
            .ok_or_else(|| {
                Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone()))
            })?;
        let working_folder_path = installed_package.package_home().await?;
        if !working_folder_path.exists() {
            return Err(Error::FsOpen(crate::error::FsOpenError::PathNotFound(
                working_folder_path,
            )));
        }

        Ok(working_folder_path)
    }

    async fn file_path(
        &self,
        namespace: &quilt_uri::Namespace,
        relative_path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let package_home = self.package_home(namespace).await?;
        let file_path = package_home.join(relative_path);
        if !file_path.exists() {
            return Err(Error::FsOpen(crate::error::FsOpenError::PathNotFound(
                file_path,
            )));
        }
        Ok(file_path)
    }

    async fn open_in_file_browser(
        &self,
        namespace: &quilt_uri::Namespace,
    ) -> Result<PathBuf, Error> {
        let dir_path = self.package_home(namespace).await?;
        opener::open_browser(&dir_path)?;
        Ok(dir_path)
    }

    async fn reveal_in_file_browser(
        &self,
        namespace: &quilt_uri::Namespace,
        path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let file_path = self.file_path(namespace, path).await?;
        opener::reveal(&file_path)?;
        Ok(file_path)
    }

    async fn open_in_default_application(
        &self,
        namespace: &quilt_uri::Namespace,
        path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let file_path = self.file_path(namespace, path).await?;
        opener::open(&file_path)?;
        Ok(file_path)
    }

    async fn resolve_manifest_uri(
        &self,
        uri: &quilt_uri::S3PackageUri,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(quilt::io::manifest::resolve_manifest_uri(
            self.get_quilt().lock().await.get_remote(),
            &uri.catalog,
            uri,
        )
        .await?)
    }
}

impl QuiltModel for Model {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain> {
        &self.quilt
    }
}

impl Model {
    pub fn create(data_dir: impl AsRef<Path>) -> Self {
        debug!("Root directory is {:?}", data_dir.as_ref());
        let quilt = quilt::LocalDomain::new(data_dir);
        Model {
            quilt: sync::Mutex::new(quilt),
        }
    }

    pub async fn set_home(
        &self,
        directory: impl AsRef<Path>,
    ) -> Result<quilt::lineage::Home, Error> {
        Ok(self.get_quilt().lock().await.set_home(directory).await?)
    }
}

mod ops;
pub use ops::*;

#[cfg(test)]
pub mod mocks;
