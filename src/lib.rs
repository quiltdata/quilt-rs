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

pub mod flow;

pub mod auth;
pub mod checksum;
mod error;
mod installed_package;
pub mod io;
pub mod lineage;
mod local_domain;
pub mod manifest;
pub mod paths;
mod perf;
pub mod uri;

#[cfg(test)]
pub mod fixtures;

pub use error::Error;
pub use installed_package::InstalledPackage;
pub use local_domain::LocalDomain;

pub type Res<T = ()> = std::result::Result<T, Error>;
