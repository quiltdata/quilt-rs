use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
use reqwest::header::ToStrError;
use thiserror::Error;
use url::Url;

use crate::io::remote::HostChecksums;
use crate::uri;
use crate::uri::Host;

#[derive(Error, Debug, PartialEq)]
pub enum S3Error {
    #[error("Failed to check object existence: {0}")]
    Exists(String),

    #[error("Failed to get object: {0}")]
    GetObject(String),

    #[error("Failed to get object attributes: {0}")]
    GetObjectAttributes(String),

    #[error("Failed to get object stream: {0}")]
    GetObjectStream(String),

    #[error("Failed to initialize S3 client: {0}")]
    Client(String),

    #[error("Failed to list objects client: {0}")]
    ListObjects(String),

    #[error("Failed to put object client: {0}")]
    PutObject(String),

    #[error("Failed to resolve object URL: {0}")]
    ResolveUrl(String),

    #[error("Failed to upload object: {0}")]
    UploadFile(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum AuthError {
    #[error("Failed to read credentials: {0}")]
    CredentialsRead(String),

    #[error("Failed to refresh credentials: {0}")]
    CredentialsRefresh(String),

    #[error("Failed to read tokens: {0}")]
    TokensRead(String),

    #[error("Failed to refresh tokens: {0}")]
    TokensRefresh(String),
}

/// The error type for this library
#[derive(Error, Debug)]
pub enum Error {
    #[error("Authentication failed for {0}: {1}")]
    Auth(Host, AuthError),

    #[error("ByteStreamError: {0}")]
    ByteStreamError(#[from] byte_stream::error::Error),

    #[error("Checksum error: {0}")]
    Checksum(String),

    #[error("Missing checksum: {0:?}")]
    ChecksumMissing(HostChecksums),

    #[error("Commit error: {0}")]
    Commit(String),

    #[error("Invalid file:// URI: {0}")]
    FileUri(Url),

    #[error("Invalid host: {0}")]
    Host(String),

    #[error("Failed to fetch host config: {0}")]
    HostConfig(String),

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

    #[error("Multibase error: {0}")]
    Multibase(#[from] multibase::Error),

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
    S3Raw(String),

    #[error("S3 error for {0:?}: {1}")]
    S3(Option<Host>, S3Error),

    #[error("Invalid S3 URI: {0}")]
    S3Uri(String),

    #[error("Table error: {0}")]
    Table(String),

    #[error("Cannot convert to string: {0}")]
    ToString(#[from] ToStrError),

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
