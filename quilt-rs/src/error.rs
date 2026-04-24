use std::path::PathBuf;
use std::path::StripPrefixError;
use std::str::Utf8Error;

use aws_smithy_types::byte_stream;
use reqwest::header::ToStrError;
use thiserror::Error;
use url::Url;

use crate::io::remote::HostChecksums;
use crate::uri::Host;
use crate::uri::Namespace;

#[derive(Error, Debug)]
#[error("S3 error{}: {kind}", .host.as_ref().map_or(String::new(), |h| format!(" for {h}")))]
pub struct S3Error {
    pub host: Option<Host>,
    #[source]
    pub kind: S3ErrorKind,
}

impl S3Error {
    pub fn new(kind: S3ErrorKind) -> Self {
        Self { host: None, kind }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self.kind, S3ErrorKind::NotFound(_))
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum S3ErrorKind {
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

    #[error("Failed to list objects: {0}")]
    ListObjects(String),

    #[error("Failed to put object: {0}")]
    PutObject(String),

    #[error("Failed to resolve object URL: {0}")]
    ResolveUrl(String),

    #[error("Failed to upload object: {0}")]
    UploadFile(String),

    #[error("S3 not found: {0}")]
    NotFound(String),

    #[error("S3 error: {0}")]
    Raw(String),

    #[error("Failed to initialize S3 Remote")]
    RemoteInit,

    #[error("Object key expected to be present")]
    ObjectKey,

    #[error("Error with upload id: {0}")]
    UploadId(String),

    #[error("Failed to read RwLock: {0}")]
    PoisonLock(String),
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

    #[error("Failed to exchange authorization code for tokens: {0}")]
    TokensExchange(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum InstallPackageError {
    #[error("The package {0} is already installed")]
    AlreadyInstalled(Namespace),

    #[error("The given package is not installed: {0}")]
    NotInstalled(Namespace),
}

#[derive(Error, Debug, PartialEq)]
pub enum InstallPathError {
    #[error("Failed to install path: {}", .0.display())]
    Install(PathBuf),

    #[error("Some paths are already installed")]
    AlreadyInstalled,

    #[error("Failed to uninstall path: {}", .0.display())]
    Uninstall(PathBuf),
}

#[derive(Error, Debug, PartialEq)]
pub enum UriError {
    #[error("Invalid file:// URI: {0}")]
    FileScheme(Url),

    #[error("Invalid host: {0}")]
    Host(String),

    #[error("Invalid URI scheme: {0}")]
    Scheme(String),

    #[error("Invalid namespace: {0}")]
    Namespace(String),

    #[error("Invalid package URI: {0}")]
    Package(String),

    #[error("Invalid S3 URI: {0}")]
    S3(String),

    #[error("Manifest path error: {0}")]
    ManifestPath(String),
}

#[derive(Error, Debug)]
pub enum ChecksumError {
    #[error("Checksum error: {0}")]
    Mismatch(String),

    #[error("Missing checksum: {0:?}")]
    Missing(HostChecksums),

    #[error("Malformed checksum: {0}")]
    Malformed(String),

    #[error("Invalid multihash: {0}")]
    InvalidMultihash(String),

    #[error("Failed to get checksum from S3: {0}")]
    NoS3Checksum(String),

    #[error("Multihash error: {0}")]
    Multihash(#[from] multihash::Error),

    #[error("Multibase error: {0}")]
    Multibase(#[from] multibase::Error),
}

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Manifest header: {0}")]
    Header(String),

    #[error("Failed to load manifest from {path}: {source}")]
    Load {
        path: PathBuf,
        source: Box<crate::Error>,
    },

    #[error("Table error: {0}")]
    Table(String),
}

#[derive(Error, Debug)]
pub enum LineageError {
    #[error("Domain lineage missing, including missing Home directory")]
    Missing,

    #[error("Domain lineage missing Home directory")]
    MissingHome,

    #[error("Failed to parse lineage file: {0}")]
    Parse(serde_json::Error),

    #[error("Operation requires a remote origin, but this is a local-only package")]
    NoRemote,
}

#[derive(Error, Debug, PartialEq)]
pub enum RemoteCatalogError {
    #[error("Workflow error: {0}")]
    Workflow(String),

    #[error("Failed to fetch host config: {0}")]
    HostConfig(String),

    #[error("S3 bucket '{0}' is not reachable — verify the bucket name")]
    BucketUnreachable(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum LoginError {
    #[error("Login required{}", .0.as_ref().map_or(String::new(), |h| format!(": {h}")))]
    Required(Option<Host>),

    #[error("Failed to get registry URL from {0}. Does {0}/config.json have it?")]
    RequiredRegistryUrl(Host),
}

#[derive(Error, Debug)]
pub enum FsError {
    #[error("Failed to read file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to write file {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to copy file from {from} to {to}: {source}")]
    Copy {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to create directory {path}: {source}")]
    DirectoryCreate {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("File not found: {path}")]
    NotFound { path: PathBuf },

    #[error("Path prefix not found: {0}")]
    PathPrefixNotFound(StripPrefixError),

    #[error("ByteStream error: {0}")]
    ByteStream(#[from] byte_stream::error::Error),
}

#[derive(Error, Debug, PartialEq)]
pub enum PackageOpError {
    #[error("Commit error: {0}")]
    Commit(String),

    #[error("Push error: {0}")]
    Push(String),

    #[error("Publish error: {0}")]
    Publish(String),

    #[error("General error regarding package: {0}")]
    Package(String),
}

/// The error type for this library
#[derive(Error, Debug)]
pub enum Error {
    #[error("Authentication failed for {0}: {1}")]
    Auth(Host, AuthError),

    #[error(transparent)]
    Checksum(#[from] ChecksumError),

    #[error(transparent)]
    Fs(#[from] FsError),

    #[error(transparent)]
    InstallPackage(InstallPackageError),

    #[error(transparent)]
    InstallPath(InstallPathError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Lineage(#[from] LineageError),

    #[error(transparent)]
    Login(#[from] LoginError),

    #[error(transparent)]
    Manifest(#[from] ManifestError),

    #[error(transparent)]
    PackageOp(#[from] PackageOpError),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error(transparent)]
    RemoteCatalog(#[from] RemoteCatalogError),

    #[error(transparent)]
    S3(#[from] S3Error),

    #[error("Cannot convert to string: {0}")]
    ToString(#[from] ToStrError),

    #[error("Integer conversion error: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),

    #[error("Unimplemented")]
    Unimplemented,

    #[error(transparent)]
    Uri(#[from] UriError),

    #[error("Error parsing URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

impl Error {
    /// Returns `true` if this error represents an S3 "not found" (NoSuchKey) response.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::S3(s3) if s3.is_not_found())
    }
}

// Compose `?` across two From hops: external error → focused enum → Error.
// Rust's `?` only runs one `From::from`, so these bridges make call sites
// keep working without `.map_err(..)`.

impl From<multihash::Error> for Error {
    fn from(err: multihash::Error) -> Self {
        Error::Checksum(ChecksumError::Multihash(err))
    }
}

impl From<multibase::Error> for Error {
    fn from(err: multibase::Error) -> Self {
        Error::Checksum(ChecksumError::Multibase(err))
    }
}

impl From<byte_stream::error::Error> for Error {
    fn from(err: byte_stream::error::Error) -> Self {
        FsError::ByteStream(err).into()
    }
}

impl From<StripPrefixError> for Error {
    fn from(err: StripPrefixError) -> Self {
        Error::Fs(FsError::PathPrefixNotFound(err))
    }
}
