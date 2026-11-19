//! A dataspace: a folder of working folders, with the contract, `data.toml`,
//! that says what should happen to the packages they hold, and the lock,
//! `data.lock.toml`, that records which revisions were used.

pub mod contract;
pub mod lock;
mod metadata;
pub mod read;
pub mod read_all;

use std::path::PathBuf;

use thiserror::Error;

use contract::Access;
pub use metadata::METADATA_FILE;

/// Why an operation on one entry of a contract was refused or failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EntryError {
    #[error("the entry is under `{0}`, which this operation does not take")]
    Access(Access),

    #[error("the entry names a local registry, which this operation does not read")]
    LocalRegistry,

    #[error(
        "the package holds a file named {METADATA_FILE}, which is where Quilt puts its metadata; move the file in the package, then try again"
    )]
    HoldsMetadataFile,

    #[error("{} changed while the operation ran, so it was left as it is", .0.display())]
    Changed(PathBuf),

    #[error(transparent)]
    Failed(#[from] Box<crate::Error>),
}

impl From<crate::Error> for EntryError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}
