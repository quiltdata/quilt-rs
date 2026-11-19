//! A dataspace: a folder of working folders, with the contract, `data.toml`,
//! that says what should happen to the packages they hold, and the lock,
//! `data.lock.toml`, that records which revisions were used.

pub mod contract;
pub mod lock;
mod metadata;
pub mod publish;
pub mod read;
pub mod read_all;
pub mod reattach;

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

    #[error(
        "the package holds a file named {METADATA_FILE}, which is where Quilt puts its metadata; move the file in the package, then try again"
    )]
    HoldsMetadataFile,

    #[error("{} changed while the operation ran, so it was left as it is", .0.display())]
    Changed(PathBuf),

    #[error("the working folder {} is missing or cannot be read, so nothing was published", .0.display())]
    FolderUnavailable(PathBuf),

    #[error(
        "the working folder {} is missing or holds none of the package's files, so it was not read as their deletion",
        .0.display()
    )]
    FolderGone(PathBuf),

    #[error(
        "the working folder {} holds no files, and `replace` would remove every file of the package",
        .0.display()
    )]
    NothingToReplace(PathBuf),

    #[error("{METADATA_FILE} is not valid: {0}")]
    MetadataFile(String),

    #[error("{METADATA_FILE} cannot hold this revision: {0}")]
    MetadataUnwritable(String),

    #[error("`latest` cannot be read, so `write` has nothing to lay its files over")]
    LatestUnreadable,

    #[error(transparent)]
    Failed(#[from] Box<crate::Error>),
}

impl From<crate::Error> for EntryError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}
