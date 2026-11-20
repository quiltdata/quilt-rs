//! A dataspace: a folder of working folders, with the contract, `data.toml`,
//! that says what should happen to the packages they hold, and the lock,
//! `data.lock.toml`, that records which revisions were used.
//!
//! Each operation is a module of its own, named for its verb, with its
//! tests in `<verb>/tests.rs`: the flows a contract runs (`read`,
//! `publish`, `sync` and their `_all` forms, `autosync`), and the commands a
//! user gives a dataspace (`install`, `uninstall`, `prune`, `reattach`,
//! `commit`, `undo_commit`, `reset_to_latest`, `certify_latest`,
//! `remove_revisions`). A command that changes `data.toml` does so through
//! `edit`, the one writer of the contract, and writes the lock last.

pub mod autosync;
pub mod certify_latest;
pub mod commit;
pub mod conflict;
pub mod contract;
mod edit;
pub mod history;
pub mod hooks;
pub mod ignore;
pub mod install;
pub mod list;
pub mod lock;
mod metadata;
pub mod prune;
pub mod publish;
pub mod publish_all;
pub mod read;
pub mod read_all;
pub mod reattach;
pub mod remove_revisions;
mod replace;
pub mod reset_to_latest;
pub mod status;
mod store;
pub mod sync;
pub mod undo_commit;
pub mod uninstall;

use std::path::PathBuf;

use thiserror::Error;

use conflict::CONFLICT_FILE;
use contract::Access;
pub use metadata::METADATA_FILE;

/// `key`'s entry under `access` in `contract`; `Err` with the access it is
/// declared under instead, the first, or with `None` where `contract` does
/// not declare it.
pub(crate) fn entry_under(
    contract: &contract::Contract,
    key: &contract::PackageKey,
    access: Access,
) -> Result<contract::Entry, Option<Access>> {
    let mut other = None;
    for entry in contract
        .entries()
        .iter()
        .filter(|entry| entry.key() == *key)
    {
        if entry.access == access {
            return Ok(entry.clone());
        }
        other.get_or_insert(entry.access);
    }
    Err(other)
}

/// Why an operation on one entry of a contract was refused or failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EntryError {
    #[error("the entry is under `{0}`, which this operation does not take")]
    Access(Access),

    /// No local registry was given, or the one given does not answer to the
    /// name the entry gives, as another machine's does not.
    #[error("the entry names a local registry that this operation does not reach")]
    LocalRegistry,

    #[error(
        "the package holds a file named {METADATA_FILE}, which is where Quilt puts its metadata; move the file in the package, then try again"
    )]
    HoldsMetadataFile,

    #[error("{} changed while the operation ran, so it was left as it is", .0.display())]
    Changed(PathBuf),

    #[error("the working folder {} is missing or cannot be read, so nothing was published", .0.display())]
    FolderUnavailable(PathBuf),

    #[error(
        "the working folder {} is missing or empty, so it was not read as the deletion of the package's files",
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

    #[error("`latest` cannot be read, so there is no revision to build on")]
    LatestUnreadable,

    /// The local commits cannot be merged onto `latest` for a reason no
    /// decision in `conflict.json` can settle.
    #[error("the local commits cannot be merged onto `latest`: {0}")]
    Unmergeable(String),

    #[error("{CONFLICT_FILE}: {0}")]
    ConflictMap(String),

    #[error(transparent)]
    Hook(#[from] hooks::HookError),

    #[error(transparent)]
    Queue(#[from] Box<crate::queue::QueueError>),

    #[error(transparent)]
    Failed(#[from] Box<crate::Error>),
}

impl From<crate::queue::QueueError> for EntryError {
    fn from(err: crate::queue::QueueError) -> Self {
        Self::Queue(Box::new(err))
    }
}

impl From<crate::Error> for EntryError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}
