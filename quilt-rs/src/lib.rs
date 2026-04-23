#![doc = include_str!("../README.md")]

pub mod flow;

pub mod auth;
pub mod checksum;
#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
mod installed_package;
pub mod io;
pub mod junk;
pub mod lineage;
mod local_domain;
pub mod manifest;
pub mod paths;
pub mod quiltignore;
pub mod uri;

#[cfg(test)]
pub mod fixtures;

pub use error::AuthError;
pub use error::ChecksumError;
pub use error::Error;
pub use error::FsError;
pub use error::InstallPackageError;
pub use error::InstallPathError;
pub use error::LineageError;
pub use error::LoginError;
pub use error::ManifestError;
pub use error::PackageOpError;
pub use error::RemoteCatalogError;
pub use error::S3Error;
pub use error::S3ErrorKind;
pub use error::UriError;
pub use installed_package::InstalledPackage;
pub use installed_package::PublishOutcome;
pub use installed_package::PushOutcome;
pub use local_domain::LocalDomain;

pub type Res<T = ()> = std::result::Result<T, Error>;
