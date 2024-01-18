use futures::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::info;

pub use crate::quilt::{
    manifest::JsonObject, Manifest, ManifestHeader, S3PackageURI, RemoteManifest, LocalDomain,
    lineage::PackageLineage, InstalledPackage, InstalledPackageStatus
};

// Types

#[derive(Clone, Debug, Serialize)]
pub struct AvailablePackage {
    pub namespace: String,
    pub lineage: PackageLineage,
    // XXX: state and stuff
}

#[allow(dead_code)]
impl AvailablePackage {
    pub async fn from_quilt(package: &InstalledPackage) -> Result<Self, String> {
        let lineage = package.lineage().await?;
        Ok(Self {
            namespace: package.namespace.clone(),
            lineage,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstalledPackageState {
    remote: RemoteManifest,
    compatible: bool,
    modified: bool, // expose changed paths? diff?
}

#[allow(dead_code)]
impl InstalledPackageState {
    pub async fn from_quilt(package: &InstalledPackage) -> Result<Self, String> {
        let lineage = package.lineage().await?;
        Ok(Self {
            remote: lineage.remote,
            compatible: false,
            modified: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstallPreflightCheck {
    // whether it's safe to automatically install the package/paths
    safe: bool,
    path_valid: bool,
    // resolved remote manifest location
    remote: RemoteManifest,
    installed: Option<InstalledPackageState>,
}

// Commands

// should return enough data to show the prompt for installing / overwriting
#[allow(dead_code)]
pub async fn install_preflight_check(
    local_domain: Mutex<LocalDomain>,
    uri: S3PackageURI,
) -> Result<InstallPreflightCheck, String> {
    // resolve the uri into a remote manifest
    let remote = RemoteManifest::resolve(&uri).await?;

    let local_domain = local_domain.lock().await;

    let installed = if let Some(package) = local_domain
        .get_installed_package(&remote.namespace)
        .await?
    {
        Some(InstalledPackageState::from_quilt(&package).await?)
    } else {
        None
    };

    let cached_manifest = local_domain.cache_remote_manifest(&remote).await?;

    let path_valid = if let Some(path) = uri.path {
        let manifest = cached_manifest.read().await?;
        manifest.has_path(&path)
    } else {
        true
    };

    Ok(InstallPreflightCheck {
        // TODO: use some heuristics for checking if it's safe to install:
        // - total size is under a certain sensible limit
        // - no conflicts / local modifications
        safe: false,
        path_valid,
        remote,
        installed,
    })
}

#[allow(dead_code)]
pub async fn install_package(
    local_domain: Mutex<LocalDomain>,
    remote: RemoteManifest,
    paths: Vec<String>,
    // force? for overwriting
) -> Result<(), String> {
    info!("install_package({remote:?}, {paths:?})");
    let installed_package = local_domain.lock().await.install_package(&remote).await?;

    installed_package.install_paths(&paths).await?;

    Ok(())
}

#[allow(dead_code)]
pub async fn install_package_paths(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
    paths: Vec<String>,
    // force? for overwriting
) -> Result<(), String> {
    info!("install_package_paths({namespace}, {paths:?})");
    local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("package not installed")?
        .install_paths(&paths)
        .await
}

#[allow(dead_code)]
pub async fn uninstall_package(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
) -> Result<(), String> {
    info!("uninstall_package({namespace:?})");
    local_domain.lock().await.uninstall_package(namespace).await
}

pub async fn browse_remote_manifest(
    local_domain: Mutex<LocalDomain>,
    remote: RemoteManifest,
) -> Result<Manifest, String> {
    info!("browse_remote_manifest({remote:?})");
    local_domain
        .lock()
        .await
        .browse_remote_manifest(&remote)
        .await
}

pub async fn browse_remote_package(
    local_domain: Mutex<LocalDomain>,
    uri: S3PackageURI,
) -> Result<Manifest, String> {
    info!("browse_remote_package({uri:?})");
    local_domain.lock().await.browse_uri(&uri).await
}

pub async fn list_installed_packages(
    local_domain: Mutex<LocalDomain>,
) -> Result<Vec<AvailablePackage>, String> {
    let packages = local_domain.lock().await.list_installed_packages().await?;
    stream::iter(packages.into_iter())
        .then(|p| async move { AvailablePackage::from_quilt(&p).await })
        .try_collect()
        .await
}

#[allow(dead_code)]
pub async fn installed_package_status(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
) -> Result<InstalledPackageStatus, String> {
    local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("not installed")?
        .status()
        .await
}

#[allow(dead_code)]
pub async fn commit(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
    message: String,
    user_meta: Option<JsonObject>,
) -> Result<(), String> {
    info!("commit('{namespace}', '{message}', {user_meta:?})");
    let package = local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("not installed")?;
    package.commit(message, user_meta).await
}

#[allow(dead_code)]
pub async fn push_package(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
) -> Result<(), String> {
    info!("push_package({namespace})");
    let package = local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("not installed")?;
    // TODO: let the caller know if diverged
    package.push().await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn pull_package(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
) -> Result<(), String> {
    info!("pull_package({namespace})");
    let package = local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("not installed")?;
    package.pull().await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn certify_latest(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
) -> Result<(), String> {
    info!("certify_latest({namespace:?})");
    local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("not installed")?
        .certify_latest()
        .await
}

#[allow(dead_code)]
pub async fn reset_to_latest(
    local_domain: Mutex<LocalDomain>,
    namespace: String,
) -> Result<(), String> {
    info!("reset_to_latest({namespace:?})");
    local_domain
        .lock()
        .await
        .get_installed_package(&namespace)
        .await?
        .ok_or("not installed")?
        .reset_to_latest()
        .await
}


