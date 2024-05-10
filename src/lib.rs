//! For all operations instantiate `LocalDomain` and then call some of its methods.
//!
//! For example, for installing package you can create path, where everything will be stored.
//! There will be `.quilt` directory and working directory for each package.
//! ```rs
//! let path = PathBuf::from("/foo/bar");
//! ```
//! Instantiate `LocalDomain` for that path .
//! ```rs
//! let local_domain = quilt_rs::LocalDomain::new(path);
//! ```
//! Create `ManifestUri`.
//! You can do this by creating "quilt+s3" URI and convert it.
//! ```rs
//! let package_uri = S3PackageUri::try_from("quilt+s3://lorem#package=ipsum@hash-is-required")?;
//! let manifest_uri = ManifestUri::try_from(package_uri)?;
//! ```
//! Then call `install_package` method. You will get `InstalledPackage` as output.
//! ```rs
//! let installed_package = local_domain.install_package(&manifest_uri).await?;
//! ```

use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
use reqwest::header::ToStrError;
use thiserror::Error;
use url::Url;

pub mod flow;

pub mod checksum;
pub mod io;
pub mod lineage;
pub mod manifest;
pub mod paths;
pub mod quilt;
pub mod uri;

#[cfg(test)]
pub mod mocks;

pub use quilt::InstalledPackage;
pub use quilt::LocalDomain;

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

    #[error("Parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    #[error("Manifest header: {0}")]
    ManifestHeader(String),

    #[error("Manifest path error: {0}")]
    ManifestPath(String),

    #[error("Invalid namespace: {0}")]
    Namespace(String),

    #[error("Cannot convert to string: {0}")]
    ToString(#[from] ToStrError),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Missing HTTP header: {0}")]
    MissingHTTPHeader(String),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),

    #[error("The package {0} is already installed")]
    PackageAlreadyInstalled(uri::Namespace),

    #[error("The given package is not installed: {0}")]
    PackageNotInstalled(uri::Namespace),

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

    #[error("Invalid file:// URI: {0}")]
    FileUri(Url),

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
