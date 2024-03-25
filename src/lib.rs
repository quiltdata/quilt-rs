mod quilt4;
mod s3_utils;

pub mod quilt;
pub mod utils;

use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
pub use quilt4::{
    manifest::Manifest4, row4::Row4, table::Table, upath::UPath, uri::UriParser, uri::UriQuilt,
};

pub use quilt::{InstalledPackage, LocalDomain, Manifest, RemoteManifest, S3PackageUri};

use reqwest::header::ToStrError;
use temp_dir::TempDir;
use thiserror::Error;

/// The error type for this library
#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Missing parent path error: {0}")]
    MissingParentPath(std::path::PathBuf),

    #[error("Failed to parse lineage file: {0}")]
    LineageParse(serde_json::Error),

    /// An error from the AWS SDK
    ///
    /// Note that this uses a string for the underlying error type, because the AWS SDK
    /// uses generic error types that are difficult to work with for downstream users.
    #[error("S3 error: {0}")]
    S3(String),

    #[error("Invalid S3 URI: {0}")]
    S3Uri(String),

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("Manifest header: {0}")]
    ManifestHeader(String),

    #[error("Manifest path error: {0}")]
    ManifestPath(String),

    #[error("Cannot convert to string: {0}")]
    ToString(#[from] ToStrError),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Missing HTTP header: {0}")]
    MissingHTTPHeader(String),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),

    #[error("The package {0} is already installed")]
    PackageAlreadyInstalled(String),

    #[error("The given package is not installed: {0}")]
    PackageNotInstalled(String),

    #[error("Failed to install path: {0}")]
    InstallPath(String),

    #[error("Uninstall error: {0}")]
    Uninstall(String),

    #[error("Invalid multihash: {0}")]
    InvalidMultihash(String),

    #[error("Multihash error: {0}")]
    Multihash(#[from] multihash::Error),

    #[error("Invalid URI scheme: {0}")]
    InvalidScheme(String),

    #[error("Invalid package URI: {0}")]
    PackageURI(String),

    #[error("General error regarding package: {0}")]
    Package(String),

    #[error("Checksum error: {0}")]
    Checksum(String),

    #[error("Error parsing URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("Table error: {0}")]
    Table(String),

    #[error("Commit error: {0}")]
    Commit(String),

    #[error("Base64 error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("Error with upload id: {0}")]
    UploadId(String),

    #[error("ByteStreamError: {0}")]
    ByteStreamError(#[from] byte_stream::error::Error),

    #[error("Unimplemented")]
    Unimplemented,
}

pub async fn install_temporarily(
    bucket: &str,
    namespace: &str,
    hash: &str,
) -> Result<InstalledPackage, Error> {
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

pub async fn installed_packages(dir: Option<String>) -> Result<Vec<InstalledPackage>, Error> {
    let path_buf = match dir {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::env::current_dir().unwrap(),
    };
    let local_domain = LocalDomain::new(path_buf);
    println!("local_domain: {:?}", local_domain);
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
    async fn test_installed_packages_in_cwd() {
        let result = installed_packages(None).await;
        assert!(result.is_ok());
        let packages = result.unwrap();
        let count = packages.len();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_installed_packages_in_test_domain() {
        let dir = crate::utils::TEST_DOMAIN.to_string();
        let result = installed_packages(Some(dir.to_string())).await;
        assert!(result.is_ok());
        let packages = result.unwrap();
        println!("packages[{}]: {:?}", crate::utils::TEST_DOMAIN, packages);
        let count = packages.len();
        assert!(count == 0); // TODO: add data.json to fix this
    }
}
