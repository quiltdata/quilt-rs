use aws_sdk_s3::Error;
use tracing::info;
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

pub use quilt::{InstalledPackage, LocalDomain, Manifest, S3PackageURI};

pub async fn manifest_from_uri(uri_string: &str) -> Result<Manifest4, Error> {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let uri = S3PackageURI::try_from(uri_string).expect("Failed to parse URI");
    let table = local_domain
        .browse_uri(&uri)
        .await
        .expect("Failed to browse remote package");
    let manifest = Manifest4::new(
        UPath::S3 {
            bucket: uri.bucket,
            path: "".into(),  // TODO
        },
        Some(table),
    );
    info!("manifest: {:#?}", manifest);
    Ok(manifest)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manifest_from_uri() {
        let uri = utils::TEST_URI_STRING;
        let manifest = manifest_from_uri(uri).await;
        assert!(manifest.is_ok());
        assert!(manifest.unwrap().table().unwrap().records.len() > 0);
    }
}
