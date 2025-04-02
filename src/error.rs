use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
use reqwest::header::ToStrError;
use thiserror::Error;
use url::Url;

use crate::uri;
use crate::uri::Host;

#[derive(Error, Debug, PartialEq)]
pub enum S3Error {
    #[error("Failed to check object existence for {0:?}: {1}")]
    Exists(Option<Host>, String),

    #[error("Failed to get object for {0:?}: {1}")]
    GetObject(Option<Host>, String),

    #[error("Failed to get object attributes for {0:?}: {1}")]
    GetObjectAttributes(Option<Host>, String),

    #[error("Failed to get object stream for {0:?}: {1}")]
    GetObjectStream(Option<Host>, String),

    #[error("Failed to initialize S3 client for {0:?}: {1}")]
    Client(Option<Host>, String),

    #[error("Failed to list objects client for {0:?}: {1}")]
    ListObjects(Option<Host>, String),

    #[error("Failed to put object client for {0:?}: {1}")]
    PutObject(Option<Host>, String),

    #[error("Failed to resolve object URL for {0:?}: {1}")]
    ResolveUrl(Option<Host>, String),

    #[error("Failed to upload object for {0:?}: {1}")]
    UploadFile(Option<Host>, String),
}

/// The error type for this library
#[derive(Error, Debug)]
pub enum Error {
    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("Base64 error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("ByteStreamError: {0}")]
    ByteStreamError(#[from] byte_stream::error::Error),

    #[error("Checksum error: {0}")]
    Checksum(String),

    #[error("Commit error: {0}")]
    Commit(String),

    #[error("Failed to read credentials for {}: {:?}", .0.0, .0.1)]
    CredentialsRead((Host, String)),

    #[error("Failed to refresh credentials for {}: {:?}", .0.0, .0.1)]
    CredentialsRefresh((Host, String)),

    #[error("Invalid file:// URI: {0}")]
    FileUri(Url),

    #[error("Invalid host: {0}")]
    Host(String),

    #[error("Failed to install path: {0}")]
    InstallPath(String),

    #[error("Invalid multihash: {0}")]
    InvalidMultihash(String),

    #[error("Invalid URI scheme: {0}")]
    InvalidScheme(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Domain lineage missing, including missing Home directory")]
    LineageMissing,

    #[error("Domain lineage missing Home directory")]
    LineageMissingHome,

    #[error("Failed to get access token: {0:?}")]
    LoginRequired(Option<Host>),

    #[error("Failed to get registry URL from {0}. Does {0}/config.json have it?")]
    LoginRequiredRegistryUrl(Host),

    #[error("Failed to parse lineage file: {0}")]
    LineageParse(serde_json::Error),

    #[error("Manifest header: {0}")]
    ManifestHeader(String),

    #[error("Manifest path error: {0}")]
    ManifestPath(String),

    #[error("Missing HTTP header: {0}")]
    MissingHTTPHeader(String),

    #[error("Missing parent path error: {0}")]
    MissingParentPath(std::path::PathBuf),

    #[error("Multihash error: {0}")]
    Multihash(#[from] multihash::Error),

    #[error("Invalid namespace: {0}")]
    Namespace(String),

    #[error("Failed to get checksum from S3: {0}")]
    NoS3Checksum(String),

    #[error("Object key expected to be present")]
    ObjectKey,

    #[error("General error regarding package: {0}")]
    Package(String),

    #[error("The package {0} is already installed")]
    PackageAlreadyInstalled(uri::Namespace),

    #[error("The given package is not installed: {0}")]
    PackageNotInstalled(uri::Namespace),

    #[error("Invalid package URI: {0}")]
    PackageURI(String),

    #[error("Parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    #[error("Path prefix not found: {0}")]
    PathPrefixNotFound(#[from] std::path::StripPrefixError),

    #[error("Failed to read RwLock: {0}")]
    PoisonLock(String),

    #[error("Push error: {0}")]
    Push(String),

    #[error("Failed to initialize S3 Remote")]
    RemoteInit,

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// An error from the AWS SDK
    ///
    /// Note that this uses a string for the underlying error type, because the AWS SDK
    /// uses generic error types that are difficult to work with for downstream users.
    #[error("S3 error: {0}")]
    S3(String),

    #[error("S3 error: {0}")]
    S3V2(S3Error),

    #[error("Invalid S3 URI: {0}")]
    S3Uri(String),

    #[error("Table error: {0}")]
    Table(String),

    #[error("Cannot convert to string: {0}")]
    ToString(#[from] ToStrError),

    #[error("Failed to read tokens for {}: {:?}", .0.0, .0.1)]
    TokensRead((Host, String)),

    #[error("Failed to refresh tokens for {0}: {1}")]
    TokensRefresh(Host, String),

    #[error("Integer conversion error: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),

    #[error("Unimplemented")]
    Unimplemented,

    #[error("Uninstall error: {0}")]
    Uninstall(String),

    #[error("Error with upload id: {0}")]
    UploadId(String),

    #[error("Error parsing URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),

    #[error("Workflow error: {0}")]
    Workflow(String),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}
