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

/// Result of checking whether a package is already installed.
pub enum InstallCheck {
    /// Exact same hash — already up to date
    AlreadyInstalled(quilt::InstalledPackage),
    /// Same namespace, different hash — needs pull.
    /// Contains the hash of the currently installed version.
    DifferentVersion(String),
    /// Installed locally without a remote origin
    LocalOnly,
    /// Not installed at all
    NotInstalled,
}

/// Result of attempting to install a package.
#[derive(Debug)]
pub enum InstallOutcome {
    /// Package was installed (or was already installed with the same hash).
    Installed(quilt::InstalledPackage),
    /// A different version is already installed.
    DifferentVersion {
        requested_hash: String,
        installed_hash: String,
    },
    /// Installed locally without a remote origin.
    LocalOnly,
}

impl InstallOutcome {
    /// Unwrap the installed package, or return an error.
    #[cfg(test)]
    fn into_installed(self) -> std::result::Result<quilt::InstalledPackage, Error> {
        match self {
            InstallOutcome::Installed(pkg) => Ok(pkg),
            other => Err(Error::Test(format!("expected Installed, got {other:?}"))),
        }
    }
}

pub struct Model {
    quilt: sync::Mutex<quilt::LocalDomain>,
}

#[automock]
pub trait QuiltModel {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain>;

    async fn browse_remote_manifest(
        &self,
        remote_manifest: &quilt::uri::ManifestUri,
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
        manifest_uri: &quilt::uri::ManifestUri,
    ) -> Result<InstallCheck, Error> {
        match self.get_installed_package(&manifest_uri.namespace).await? {
            Some(installed_package) => {
                let package_lineage = self
                    .get_installed_package_lineage(&installed_package)
                    .await?;
                let installed_manifest_uri = match package_lineage.remote_uri.as_ref() {
                    Some(uri) => uri,
                    None => return Ok(InstallCheck::LocalOnly),
                };
                if manifest_uri.hash == installed_manifest_uri.hash {
                    Ok(InstallCheck::AlreadyInstalled(installed_package))
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

    async fn set_origin(
        &self,
        package: &quilt::InstalledPackage,
        origin: quilt::uri::Host,
    ) -> Result<(), Error> {
        Ok(package.set_origin(origin).await?)
    }

    async fn set_remote(
        &self,
        package: &quilt::InstalledPackage,
        origin: quilt::uri::Host,
        bucket: String,
    ) -> Result<(), Error> {
        Ok(package.set_remote(bucket, Some(origin)).await?)
    }

    async fn package_create(
        &self,
        namespace: quilt::uri::Namespace,
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
            .ok_or_else(|| Error::Quilt(quilt::Error::InstallPackage(quilt::InstallPackageError::NotInstalled(namespace.clone()))))?;
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

    async fn resolve_manifest_uri(
        &self,
        uri: &quilt::uri::S3PackageUri,
    ) -> Result<quilt::uri::ManifestUri, Error> {
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
        .ok_or_else(|| Error::Quilt(quilt::Error::InstallPackage(quilt::InstallPackageError::NotInstalled(namespace))))?;

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

pub async fn install_paths(
    model: &impl QuiltModel,
    installed_package: &quilt::InstalledPackage,
    paths: Vec<PathBuf>,
) -> Result<PathBuf, Error> {
    if paths.is_empty() {
        return Err(Error::General(
            "Cannot install paths: empty paths vector provided".to_string(),
        ));
    }

    let namespace = &installed_package.namespace;

    model
        .package_install_paths(installed_package, &paths)
        .await?;

    // Post-installation actions based on number of paths
    match paths.len() {
        1 => {
            let path = &paths[0];
            info!("Installed {:?}", path);
            model.reveal_in_file_browser(namespace, path).await
        }
        _ => {
            info!("Installed {} paths", paths.len());
            model.open_in_file_browser(namespace).await
        }
    }
}

pub async fn install_package_only(
    model: &impl QuiltModel,
    uri: &quilt::uri::S3PackageUri,
) -> Result<InstallOutcome, Error> {
    let manifest_uri = model.resolve_manifest_uri(uri).await?;

    match model.is_package_installed(&manifest_uri).await? {
        InstallCheck::AlreadyInstalled(installed_package) => {
            debug!("Package already installed: {:?}", manifest_uri.namespace);
            Ok(InstallOutcome::Installed(installed_package))
        }
        InstallCheck::DifferentVersion(installed_hash) => {
            debug!(
                "Different version already installed: {:?}",
                manifest_uri.namespace
            );
            Ok(InstallOutcome::DifferentVersion {
                requested_hash: manifest_uri.hash.clone(),
                installed_hash,
            })
        }
        InstallCheck::LocalOnly => {
            debug!(
                "Local-only package already installed: {:?}",
                manifest_uri.namespace
            );
            Ok(InstallOutcome::LocalOnly)
        }
        InstallCheck::NotInstalled => {
            debug!("Package not installed, installing: {:?}", manifest_uri);
            Ok(InstallOutcome::Installed(
                model.package_install(&manifest_uri).await?,
            ))
        }
    }
}

pub async fn install_paths_only(
    model: &impl QuiltModel,
    namespace: &quilt::uri::Namespace,
    paths: Vec<PathBuf>,
) -> Result<PathBuf, Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::General("Package not found for path installation".to_string()))?;

    install_paths(model, &installed_package, paths).await
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

pub async fn set_origin(
    model: &impl QuiltModel,
    namespace: &quilt::uri::Namespace,
    origin: quilt::uri::Host,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::Quilt(quilt::Error::InstallPackage(quilt::InstallPackageError::NotInstalled(namespace.clone()))))?;
    model.set_origin(&installed_package, origin).await?;
    Ok(())
}

pub async fn set_remote(
    model: &impl QuiltModel,
    namespace: &quilt::uri::Namespace,
    origin: quilt::uri::Host,
    bucket: String,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::Quilt(quilt::Error::InstallPackage(quilt::InstallPackageError::NotInstalled(namespace.clone()))))?;
    model.set_remote(&installed_package, origin, bucket).await?;
    Ok(())
}

pub async fn package_create(
    model: &impl QuiltModel,
    namespace: quilt::uri::Namespace,
    source: Option<PathBuf>,
    message: Option<String>,
) -> Result<quilt::InstalledPackage, Error> {
    model.package_create(namespace, source, message).await
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

pub async fn login_oauth(
    model: &impl QuiltModel,
    host: &quilt::uri::Host,
    params: quilt::auth::OAuthParams,
) -> Result<(), Error> {
    model
        .get_quilt()
        .lock()
        .await
        .get_remote()
        .login_oauth(host, params)
        .await?;
    Ok(())
}

pub async fn get_or_register_client(
    model: &impl QuiltModel,
    host: &quilt::uri::Host,
    redirect_uri: &str,
) -> Result<String, Error> {
    let client = model
        .get_quilt()
        .lock()
        .await
        .get_remote()
        .get_or_register_client(host, redirect_uri)
        .await?;
    Ok(client.client_id)
}

#[cfg(test)]
pub mod mocks {
    use super::*;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use crate::quilt;
    use crate::Result;

    pub fn create() -> MockQuiltModel {
        MockQuiltModel::new()
    }

    pub fn create_remote_manifest() -> quilt::manifest::Manifest {
        quilt::manifest::Manifest {
            header: quilt::manifest::ManifestHeader {
                version: "v0".to_string(),
                message: None,
                user_meta: None,
                workflow: None,
            },
            rows: Vec::new(),
        }
    }

    pub fn mock_installed_package(model: &mut MockQuiltModel) -> &MockQuiltModel {
        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
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
                quilt::manifest::ManifestRow::default(),
            )]))
        });
        model
            .expect_browse_remote_manifest()
            .returning(|_| Ok(create_remote_manifest()));
        model
    }

    pub fn mock_remote_package(model: &mut MockQuiltModel) -> &MockQuiltModel {
        // Mock resolve_manifest_uri to return the manifest URI directly
        model
            .expect_resolve_manifest_uri()
            .returning(|uri| Ok(quilt::uri::ManifestUri::try_from(uri.clone()).unwrap()));
        // For the remote package test, the package starts as not installed
        model
            .expect_is_package_installed()
            .returning(|_| Ok(InstallCheck::NotInstalled));
        // After installation, the package should be available
        model.expect_get_installed_package().returning(|_| {
            Ok(Some(
                quilt::LocalDomain::new(PathBuf::new())
                    .create_installed_package(("foo", "bar").into())
                    .expect("Failed to create installed package"),
            ))
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
        model.expect_is_path_installed().returning(|_, _| Ok(false));
        model
            .expect_package_home()
            .returning(|_| Ok(PathBuf::default()));
        model
            .expect_open_in_default_application()
            .returning(|_, _| Ok(PathBuf::default()));
        model
            .expect_reveal_in_file_browser()
            .returning(|_, _| Ok(PathBuf::default()));
        model
            .expect_open_in_file_browser()
            .returning(|_| Ok(PathBuf::default()));

        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22".to_string(),
            origin: None,
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
                quilt::manifest::ManifestRow::default(),
            )]))
        });

        model
    }

    /// Mock for the case where the package is already installed with a different hash.
    pub fn mock_remote_package_different_version(model: &mut MockQuiltModel) -> &MockQuiltModel {
        model
            .expect_resolve_manifest_uri()
            .returning(|uri| Ok(quilt::uri::ManifestUri::try_from(uri.clone()).unwrap()));
        model
            .expect_is_package_installed()
            .returning(|_| Ok(InstallCheck::DifferentVersion("aaaa1111".to_string())));

        // These are needed for ViewInstalledPackage::create after the error is caught
        model.expect_get_installed_package().returning(|_| {
            Ok(Some(
                quilt::LocalDomain::new(PathBuf::new())
                    .create_installed_package(("foo", "bar").into())
                    .expect("Failed to create installed package"),
            ))
        });

        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "aaaa1111".to_string(),
            origin: None,
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
                quilt::manifest::ManifestRow::default(),
            )]))
        });

        model
    }

    pub fn mock_remote_package_local_only(model: &mut MockQuiltModel) -> &MockQuiltModel {
        model
            .expect_resolve_manifest_uri()
            .returning(|uri| Ok(quilt::uri::ManifestUri::try_from(uri.clone()).unwrap()));
        model
            .expect_is_package_installed()
            .returning(|_| Ok(InstallCheck::LocalOnly));
        model.expect_get_installed_package().returning(|_| {
            Ok(Some(
                quilt::LocalDomain::new(PathBuf::new())
                    .create_installed_package(("foo", "bar").into())
                    .expect("Failed to create installed package"),
            ))
        });

        model
            .expect_get_installed_package_lineage()
            .returning(move |_| Ok(quilt::lineage::PackageLineage::default()));
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model.expect_get_installed_package_records().returning(|_| {
            Ok(BTreeMap::from([(
                PathBuf::from("NAME"),
                quilt::manifest::ManifestRow::default(),
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

    #[tokio::test]
    async fn test_install_package_only_with_timestamp_tag() -> Result {
        crate::env::init();

        let temp_dir = TempDir::new()?;
        let model = super::Model::create(temp_dir.path());
        model.set_home(temp_dir.path()).await?;

        // Use timestamp tag instead of "latest" for stable testing
        // Timestamp 1740761585 represents a specific tagged revision
        let uri = quilt::uri::S3PackageUri::try_from(
            "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs:1740761585",
        )?;

        let installed_package = install_package_only(&model, &uri).await?.into_installed()?;
        assert_eq!(
            installed_package.namespace.to_string(),
            "reference/quilt-rs"
        );

        let lineage = model
            .get_installed_package_lineage(&installed_package)
            .await?;
        assert_eq!(
            lineage.remote()?.hash,
            "a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_install_package_only_with_hash() -> Result {
        crate::env::init();

        let temp_dir = TempDir::new()?;
        let model = super::Model::create(temp_dir.path());
        model.set_home(temp_dir.path()).await?;

        let uri = quilt::uri::S3PackageUri::try_from("quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs@a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7")?;

        let first_install = install_package_only(&model, &uri).await?.into_installed()?;
        assert_eq!(first_install.namespace.to_string(), "reference/quilt-rs");

        let first_hash = model
            .get_installed_package_lineage(&first_install)
            .await?
            .remote()?
            .hash
            .clone();

        // TODO: make sure there was no double installation
        let second_install = install_package_only(&model, &uri).await?.into_installed()?;
        assert_eq!(second_install.namespace.to_string(), "reference/quilt-rs");

        let second_hash = model
            .get_installed_package_lineage(&second_install)
            .await?
            .remote()?
            .hash
            .clone();

        assert_eq!(first_hash, second_hash);
        assert_eq!(
            first_install.package_home().await?,
            second_install.package_home().await?
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_install_package_only_resolution_failure() -> Result {
        crate::env::init();

        let temp_dir = TempDir::new()?;
        let model = super::Model::create(temp_dir.path());
        // Set up home directory (required for Model to work properly)
        model.set_home(temp_dir.path()).await?;

        let uri = quilt::uri::S3PackageUri::try_from(
            "quilt+s3://nonexisting-bucket#package=two/files:latest",
        )?;

        let result = install_package_only(&model, &uri).await;
        assert!(result.is_err());
        // This error description doesn't make sense, but it is correct so far
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing HTTP header: x-amz-bucket-region"));

        Ok(())
    }

    #[tokio::test]
    async fn test_install_package_only_local_only() -> Result {
        let mut model = create();
        mock_remote_package_local_only(&mut model);

        let uri = quilt::uri::S3PackageUri::try_from(
            "quilt+s3://quilt-example#package=foo/bar@some_hash",
        )?;

        let result = install_package_only(&model, &uri).await?;
        assert!(
            matches!(result, InstallOutcome::LocalOnly),
            "expected LocalOnly, got {result:?}",
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_install_package_only_different_version() -> Result {
        let mut model = create();
        mock_remote_package_different_version(&mut model);

        let uri = quilt::uri::S3PackageUri::try_from(
            "quilt+s3://quilt-example#package=foo/bar@bbbb2222",
        )?;

        let result = install_package_only(&model, &uri).await?;
        match result {
            InstallOutcome::DifferentVersion {
                requested_hash,
                installed_hash,
            } => {
                assert_eq!(requested_hash, "bbbb2222");
                assert_eq!(installed_hash, "aaaa1111");
            }
            other => panic!("expected DifferentVersion, got {other:?}"),
        }

        Ok(())
    }
}
