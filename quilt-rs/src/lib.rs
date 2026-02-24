#![doc = include_str!("../README.md")]

pub mod flow;

pub mod auth;
pub mod checksum;
pub mod error;
mod installed_package;
pub mod io;
pub mod lineage;
mod local_domain;
pub mod manifest;
pub mod paths;
pub mod uri;

#[cfg(test)]
pub mod fixtures;

pub use error::Error;
pub use installed_package::InstalledPackage;
pub use local_domain::LocalDomain;

pub type Res<T = ()> = std::result::Result<T, Error>;
