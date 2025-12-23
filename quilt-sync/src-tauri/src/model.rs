use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mockall::predicate::*;
use mockall::*;

use tokio::sync;

use tokio_stream::StreamExt;

use crate::error::Error;
use crate::quilt;
use crate::telemetry::prelude::*;

use quilt_rs::io::remote::HostConfig;

pub struct Model {
    quilt: sync::Mutex<quilt::LocalDomain>,
}

#[automock]
pub trait QuiltModel {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain>;

    async fn browse_remote_manifest(
        &self,
        remote_manifest: &quilt::uri::ManifestUri,
    ) -> Result<quilt::manifest::Table, Error> {
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
        namespace: &quilt::uri::Namespace,
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
    ) -> Result<BTreeMap<PathBuf, quilt::manifest::Row>, Error> {
        let mut stream = package.manifest().await?.records_stream().await;
        let mut records = BTreeMap::new();
        while let Some(page) = stream.next().await {
            if let Ok(rows) = page {
                for row in rows.into_iter().flatten() {
                    records.insert(row.name.clone(), row);
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

    async fn package_commit(
        &self,
        package: &quilt::InstalledPackage,
        message: String,
        metadata: Option<serde_json::Value>,
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
    ) -> Result<quilt::uri::ManifestUri, Error> {
        Ok(package.pull(host_config).await?)
    }

    async fn is_package_installed(
        &self,
        uri: &quilt::uri::S3PackageUri,
    ) -> Result<Option<quilt::InstalledPackage>, Error> {
        match self.get_installed_package(&uri.namespace).await? {
            Some(installed_package) => {
                let package_lineage = self
                    .get_installed_package_lineage(&installed_package)
                    .await?;
                let manifest_uri = package_lineage.remote;
                let same_hash = match &uri.revision {
                    quilt::uri::RevisionPointer::Hash(h) => h == &manifest_uri.hash,
                    _ => false,
                };
                if same_hash {
                    Ok(Some(installed_package))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
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
    ) -> Result<quilt::uri::ManifestUri, Error> {
        Ok(package.push(host_config).await?)
    }

    async fn package_revision_certify_latest(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt::uri::ManifestUri, Error> {
        Ok(package.certify_latest().await?)
    }

    async fn package_revision_reset_local(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt::uri::ManifestUri, Error> {
        Ok(package.reset_to_latest().await?)
    }

    async fn package_install(
        &self,
        remote_manifest: &quilt::uri::ManifestUri,
    ) -> Result<quilt::InstalledPackage, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .install_package(remote_manifest)
            .await?)
    }

    async fn package_uninstall(&self, namespace: quilt::uri::Namespace) -> Result<(), Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .uninstall_package(namespace)
            .await?)
    }

    async fn package_home(&self, namespace: &quilt::uri::Namespace) -> Result<PathBuf, Error> {
        let installed_package = self
            .get_installed_package(namespace)
            .await?
            .ok_or_else(|| Error::Quilt(quilt::Error::PackageNotInstalled(namespace.clone())))?;
        let working_folder_path = installed_package.package_home().await?;
        if !working_folder_path.exists() {
            return Err(Error::PathNotFound(working_folder_path));
        }

        Ok(working_folder_path)
    }

    async fn file_path(
        &self,
        namespace: &quilt::uri::Namespace,
        relative_path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let package_home = self.package_home(namespace).await?;
        let file_path = package_home.join(relative_path);
        if !file_path.exists() {
            return Err(Error::PathNotFound(file_path));
        }
        Ok(file_path)
    }

    async fn open_in_file_browser(
        &self,
        namespace: &quilt::uri::Namespace,
    ) -> Result<PathBuf, Error> {
        let dir_path = self.package_home(namespace).await?;
        opener::open_browser(&dir_path)?;
        Ok(dir_path)
    }

    async fn reveal_in_file_browser(
        &self,
        namespace: &quilt::uri::Namespace,
        path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let file_path = self.file_path(namespace, path).await?;
        opener::reveal(&file_path)?;
        Ok(file_path)
    }

    async fn open_in_default_application(
        &self,
        namespace: &quilt::uri::Namespace,
        path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let file_path = self.file_path(namespace, path).await?;
        opener::open(&file_path)?;
        Ok(file_path)
    }
}

impl QuiltModel for Model {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain> {
        &self.quilt
    }
}

impl Model {
    pub fn create(data_dir: PathBuf) -> Self {
        debug!("Root directory is {:?}", data_dir);
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

fn parse_metadata(input: &str) -> Result<Option<serde_json::Value>, Error> {
    if input.is_empty() {
        return Ok(None);
    }
    let metadata_json: Result<serde_json::Value, serde_json::Error> = serde_json::from_str(input);
    match metadata_json {
        Ok(json) => Ok(Some(json)),
        Err(err) => Err(Error::Json(err)),
    }
}

pub async fn package_commit(
    model: &impl QuiltModel,
    namespace: quilt::uri::Namespace,
    message: &str,
    metadata: &str,
    workflow: Option<String>,
    host_config: Option<HostConfig>,
) -> Result<(), Error> {
    debug!(
        "Committing the package.\nNamespace:\n{},\nmessage: {},\nuser_meta: {},\nworkflow: {:?}",
        namespace, message, metadata, workflow
    );
    let metadata = parse_metadata(metadata)?;

    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::Quilt(quilt::Error::PackageNotInstalled(namespace)))?;

    let workflow = installed_package.resolve_workflow(workflow).await?;
    model
        .package_commit(
            &installed_package,
            message.to_string(),
            metadata,
            workflow,
            host_config,
        )
        .await?;
    Ok(())
}

/// Represents the type of installation being performed
#[derive(Debug)]
pub enum PathsInstallation {
    /// A single file from a deep link
    DeepLink(PathBuf),
    /// A single file selected by the user
    SingleFile(PathBuf),
    /// Multiple files selected by the user
    Bulk(Vec<PathBuf>),
}

/// Determines what files to install based on either URI path or explicit paths list
///
/// This function handles two mutually exclusive cases:
/// 1. URI contains a path - use that single path (deep link case)
/// 2. Explicit paths are provided - use those paths
///
/// Returns None if no valid paths are found to install
fn how_many_files_to_install(
    uri: &quilt::uri::S3PackageUri,
    paths: Option<Vec<PathBuf>>,
) -> Option<PathsInstallation> {
    if uri.path.is_some() && paths.is_some() {
        error!("Both URI path and explicit paths provided. Using only URI path.");
    }

    // Case 1: URI has a path (deep link)
    if let Some(path) = &uri.path {
        return Some(PathsInstallation::DeepLink(path.clone()));
    }

    // Case 2: Explicit paths provided
    match paths {
        Some(paths) if paths.is_empty() => None,
        Some(paths) if paths.len() == 1 => Some(PathsInstallation::SingleFile(paths[0].clone())),
        Some(paths) => Some(PathsInstallation::Bulk(paths)),
        None => None,
    }
}

pub async fn install_paths(
    model: &impl QuiltModel,
    installed_package: &quilt::InstalledPackage,
    condition: PathsInstallation,
) -> Result<(), Error> {
    let namespace = &installed_package.namespace;
    match &condition {
        PathsInstallation::DeepLink(path) => {
            info!("Install {:?} via deep link", path);
            model
                .package_install_paths(installed_package, std::slice::from_ref(path))
                .await?;
            model.open_in_default_application(namespace, path).await?;
        }
        PathsInstallation::SingleFile(path) => {
            info!("Installing {:?}", path);
            model
                .package_install_paths(installed_package, std::slice::from_ref(path))
                .await?;
            model.reveal_in_file_browser(namespace, path).await?;
        }
        PathsInstallation::Bulk(paths) => {
            info!("Installing {} paths", paths.len(),);
            model
                .package_install_paths(installed_package, paths)
                .await?;
            model.open_in_file_browser(namespace).await?;
        }
    };
    Ok(())
}

pub async fn package_install(
    model: &impl QuiltModel,
    uri: &quilt::uri::S3PackageUri,
    paths: Option<Vec<PathBuf>>,
) -> Result<quilt::InstalledPackage, Error> {
    let installed_package = match model.get_installed_package(&uri.namespace).await? {
        Some(installed_package) => installed_package,
        None => {
            debug!("Installing the package: {:?}", uri);
            let manifest_uri = quilt::uri::ManifestUri::try_from(uri.clone())?;
            model.package_install(&manifest_uri).await?
        }
    };

    if let Some(paths_condition) = how_many_files_to_install(uri, paths) {
        install_paths(model, &installed_package, paths_condition).await?;
    }

    Ok(installed_package)
}

pub async fn package_uninstall(
    model: &impl QuiltModel,
    namespace: quilt::uri::Namespace,
) -> Result<(), Error> {
    debug!("Uninstall package for {} namespace", &namespace);
    model.package_uninstall(namespace).await?;
    Ok(())
}

pub fn open_in_web_browser(url: &str) -> Result<(), Error> {
    Ok(opener::open(url)?)
}

pub async fn package_revision_certify_latest(
    model: &impl QuiltModel,
    namespace: quilt::uri::Namespace,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model
        .package_revision_certify_latest(&installed_package)
        .await?;
    Ok(())
}

pub async fn package_revision_reset_local(
    model: &impl QuiltModel,
    namespace: quilt::uri::Namespace,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model
        .package_revision_reset_local(&installed_package)
        .await?;
    Ok(())
}

pub async fn package_push(
    model: &impl QuiltModel,
    namespace: &quilt::uri::Namespace,
    host_config: Option<HostConfig>,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model.package_push(&installed_package, host_config).await?;
    Ok(())
}

pub async fn package_pull(
    model: &impl QuiltModel,
    namespace: &quilt::uri::Namespace,
    host_config: Option<HostConfig>,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model.package_pull(&installed_package, host_config).await?;
    Ok(())
}

pub async fn login(
    model: &impl QuiltModel,
    host: &quilt::uri::Host,
    code: String,
) -> Result<(), Error> {
    model
        .get_quilt()
        .lock()
        .await
        .get_remote()
        .login(host, code)
        .await?;
    Ok(())
}

#[cfg(test)]
pub mod mocks {
    use super::MockQuiltModel;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::quilt;

    pub fn create() -> MockQuiltModel {
        MockQuiltModel::new()
    }

    pub fn create_remote_manifest() -> quilt::manifest::Table {
        quilt::manifest::Table::default()
    }

    pub fn mock_installed_package(model: &mut MockQuiltModel) -> &MockQuiltModel {
        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22".to_string(),
            catalog: None,
        };
        model.expect_get_installed_package().returning(move |_| {
            Ok(Some(
                quilt::LocalDomain::new(PathBuf::new())
                    .create_installed_package(("foo", "bar").into())
                    .expect("Failed to create installed package"),
            ))
        });
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model.expect_get_installed_package_records().returning(|_| {
            Ok(BTreeMap::from([(
                PathBuf::from("NAME"),
                quilt::manifest::Row::default(),
            )]))
        });
        model
            .expect_browse_remote_manifest()
            .returning(|_| Ok(create_remote_manifest()));
        model
    }

    pub fn mock_remote_package(model: &mut MockQuiltModel) -> &MockQuiltModel {
        let mut a = 0;
        // let installed_package = quilt::LocalDomain::new(PathBuf::new())
        //     .create_installed_package(("foo", "bar").into())
        //     .expect("Failed to create installed package");
        model.expect_is_package_installed().returning(|_| Ok(None));
        model.expect_get_installed_package().returning(move |_| {
            if a == 0 {
                a += 1;
                Ok(None)
            } else {
                Ok(Some(
                    quilt::LocalDomain::new(PathBuf::new())
                        .create_installed_package(("foo", "bar").into())
                        .expect("Failed to create installed package"),
                ))
            }
        });
        model.expect_package_install().returning(|_| {
            Ok(quilt::LocalDomain::new(PathBuf::new())
                .create_installed_package(("foo", "bar").into())
                .expect("Failed to create installed package"))
        });
        model.expect_package_install_paths().returning(|_, paths| {
            let mut lineage_paths = BTreeMap::new();
            for path in paths {
                lineage_paths.insert(path.clone(), quilt::lineage::PathState::default());
            }
            Ok(lineage_paths)
        });
        model
            .expect_package_home()
            .returning(|_| Ok(PathBuf::default()));
        model
            .expect_open_in_default_application()
            .returning(|_, _| Ok(PathBuf::default()));
        model
            .expect_open_in_file_browser()
            .returning(|_| Ok(PathBuf::default()));

        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22".to_string(),
            catalog: None,
        };

        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model.expect_get_installed_package_records().returning(|_| {
            Ok(BTreeMap::from([(
                PathBuf::from("NAME"),
                quilt::manifest::Row::default(),
            )]))
        });

        model
    }

    pub fn mock_installed_packages_list(model: &mut MockQuiltModel) -> &MockQuiltModel {
        model
            .expect_get_installed_packages_list()
            .returning(|| Ok(Vec::new()));
        model
    }
}
