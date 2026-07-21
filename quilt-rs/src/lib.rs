#![doc = include_str!("../README.md")]
// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]
// Test fns return `Res` and end in `Ok(())` so the body can use `?`; that trips
// `unnecessary_wraps` in tests only. Enforce it in production, allow under test.
#![cfg_attr(test, allow(clippy::unnecessary_wraps))]

pub mod flow;

pub mod auth;
pub mod checksum;
pub mod error;
mod installed_package;
pub mod io;
pub mod junk;
pub mod lineage;
mod local_domain;
pub mod manifest;
pub mod object_hash;
pub mod paths;
pub mod quiltignore;

pub mod workflow;

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
pub use installed_package::InstalledPackage;
pub use installed_package::PublishOutcome;
pub use installed_package::PushOutcome;
pub use installed_package::SetRemoteOutcome;
pub use local_domain::LocalDomain;
pub use workflow::WorkflowValidationError;

pub type Res<T = ()> = std::result::Result<T, Error>;
