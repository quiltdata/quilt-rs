pub mod api;
pub mod quilt4;
pub mod s3_utils;

pub mod quilt;
pub mod data_yaml;

pub use quilt4:: {
    string_map::StringMap,
    client::{Client, GetClient},
    domain::Domain,
    entry::Entry4,
    manifest::Manifest4,
    namespace::Namespace,
    table::Table,
    row4::Row4,
    upath::UPath,
    uri::UriParser,
    uri::UriQuilt,
};

pub use api::LocalDomain;
pub use api::Manifest;
pub use api::ManifestHeader;

pub use quilt::ManifestRow;
pub use api::AvailablePackage;
pub use quilt::InstalledPackage;
pub use api::RemoteManifest;
pub use api::S3PackageURI;
pub use api::PackageLineage;

pub use api::browse_remote_package;
pub use api::browse_remote_manifest;
pub use api::list_installed_packages;

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
    println!("remote_manifest: {:?}", remote_manifest);

    let result = loc.install_package(&remote_manifest).await;
    println!("result: {:?}", result);
    result
}

pub async fn installed_packages() -> Result<Vec<AvailablePackage>, String> {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let installed_packages: Vec<AvailablePackage> = list_installed_packages(local_domain.into())
        .await
        .expect("Failed to list installed packages");
    Ok(installed_packages)
}
