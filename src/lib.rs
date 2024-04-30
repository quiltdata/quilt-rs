use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
use reqwest::header::ToStrError;
use thiserror::Error;

mod checksum;
mod flow;
mod paths;

pub mod io;
pub mod lineage;
pub mod manifest;
pub mod quilt;
pub mod uri;

#[cfg(test)]
/// Utilities for testing only
mod utils; // FIXME: move to mocks and fixtures

pub use manifest::Row;
pub use manifest::Table;

pub use flow::status::DiscreteChange;
pub use flow::status::InstalledPackageStatus;
pub use flow::status::PackageFileFingerprint;
pub use flow::status::UpstreamDiscreteState;
pub use quilt::InstalledPackage;
pub use quilt::LocalDomain;
pub use quilt::Manifest;
pub use quilt::ManifestUri;
pub use quilt::Namespace;
pub use quilt::S3PackageUri;

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
    PackageAlreadyInstalled(Namespace),

    #[error("The given package is not installed: {0}")]
    PackageNotInstalled(Namespace),

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
