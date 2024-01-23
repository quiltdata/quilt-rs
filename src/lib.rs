mod quilt4;
mod s3_utils;

pub mod data_yaml;
pub mod quilt;

pub use quilt4::{
    client::{Client, GetClient},
    domain::Domain,
    entry::Entry4,
    manifest::Manifest4,
    namespace::Namespace,
    row4::Row4,
    string_map::StringMap,
    table::Table,
    upath::UPath,
    uri::UriParser,
    uri::UriQuilt,
};

pub use quilt::{InstalledPackage, LocalDomain, Manifest, RemoteManifest, S3PackageURI};

use temp_dir::TempDir;

pub async fn install_temporarily(
    bucket: &str,
    namespace: &str,
    hash: &str,
) -> Result<InstalledPackage, String> {
    let temp_folder = TempDir::new().unwrap();
    let loc = LocalDomain::new(temp_folder.path().to_path_buf());
    let remote_manifest = RemoteManifest {
        bucket: bucket.to_string(),
        namespace: namespace.to_string(),
        hash: hash.to_string(),
    };
    tracing::info!("remote_manifest: {:?}", remote_manifest);

    let result = loc.install_package(&remote_manifest).await;
    tracing::info!("result: {:?}", result);
    result
}

pub async fn installed_packages() -> Result<Vec<InstalledPackage>, String> {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let installed_packages = local_domain
        .list_installed_packages()
        .await
        .expect("Failed to list installed packages");
    Ok(installed_packages)
}

