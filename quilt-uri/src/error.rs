use std::str::Utf8Error;

use thiserror::Error;
use url::ParseError;
use url::Url;

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

    #[error("Invalid tag: {0}")]
    Tag(String),

    #[error("Error parsing URL: {0}")]
    UrlParse(#[from] ParseError),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),
}
