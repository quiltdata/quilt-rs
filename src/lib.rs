use aws_sdk_s3::Error; // Import the Error type from the aws_sdk_s3 crate
use tracing::info;
mod api;
mod quilt4;
mod s3_utils;

pub mod quilt;
pub mod data_yaml;

pub use quilt4:: {
    client::Client,
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

pub async fn manifest_from_uri(uri_string: &str) -> Result<Manifest, Error> {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let uri = S3PackageURI::try_from(uri_string).expect("Failed to parse URI");
    let manifest: Manifest = browse_remote_package(local_domain.into(), uri)
        .await
        .expect("Failed to browse remote package");
    info!("manifest: {:#?}", manifest);
    assert!(manifest.rows.len() > 0);
    manifest.rows.len();
    Ok(manifest)
}

pub async fn installed_packages() -> Result<Vec<AvailablePackage>, String> {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let installed_packages: Vec<AvailablePackage> = list_installed_packages(local_domain.into())
        .await
        .expect("Failed to list installed packages");
    Ok(installed_packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manifest_from_uri() {
        let uri = utils::TEST_URI_STRING;
        let manifest = manifest_from_uri(uri).await;
        assert!(manifest.is_ok());
        assert!(manifest.unwrap().rows.len() > 0);
    }
}