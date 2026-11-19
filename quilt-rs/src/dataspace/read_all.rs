//! Every `read` entry of a dataspace, read into its working folder with no
//! Domain, and the lock written after.

use std::path::Path;

use chrono::SubsecRound;
use chrono::Utc;
use thiserror::Error;

use crate::dataspace::EntryError;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read::read;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;

/// How one `read` entry's read ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum EntryRead {
    /// The read ran. The lock took its base where [`ReadOutcome::lock_base`]
    /// names one.
    Ran(ReadOutcome),
    /// The lock names local commits of the package that are not pushed. They
    /// are in a Domain, so the package was not read.
    Unpushed,
    /// The read stopped; the lock's entry for the package was left.
    Failed(EntryError),
}

impl EntryRead {
    /// Whether the package's read completed, so the lock took its base.
    #[must_use]
    pub fn completed(&self) -> bool {
        matches!(self, Self::Ran(outcome) if outcome.lock_base().is_some())
    }
}

/// What [`read_all`] did, entry by entry.
#[derive(Debug)]
pub struct DataspaceRead {
    /// Each `read` entry, in the contract's order, with how its read ended.
    pub entries: Vec<(Entry, EntryRead)>,
}

impl DataspaceRead {
    #[must_use]
    pub fn completed(&self) -> bool {
        self.entries.iter().all(|(_, read)| read.completed())
    }
}

/// Why a dataspace was not read at all.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReadAllError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}, and a working folder holds files or the lock names local commits not pushed; remove the lock, or fix it, to read",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),
}

impl From<ContractError> for ReadAllError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for ReadAllError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

/// Reads each `read` entry of the dataspace in the folder `dataspace` into its
/// working folder. Nothing advances `latest`.
///
/// A working folder that is missing or empty is filled at the revision the
/// lock names, unless the contract names one; the next read moves it on.
///
/// The lock is then written where any read completed: each such package takes
/// the revision as its base, and `filled` is set once every entry that should
/// be read has been.
///
/// A lock written for another `data.toml` is used: the contract was edited
/// here. One written for another folder, host or user is used only while no
/// entry's working folder holds files and it names no unpushed commits;
/// otherwise nothing is read or written. A read has no Domain, so the local
/// registry a lock names is not compared, and is kept unless the lock is
/// another's.
pub async fn read_all(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    dataspace: &Path,
    writer: &Writer,
) -> Result<DataspaceRead, ReadAllError> {
    let contract = Contract::read(storage, dataspace).await?;
    let (mut lock, writer) = match Lock::read(storage, &contract).await? {
        Some(mut lock) => {
            let local = Writer {
                local_registry: lock.header.writer.local_registry.clone(),
                ..writer.clone()
            };
            let foreign: Vec<Mismatch> = lock
                .mismatches(&contract, &local)
                .into_iter()
                .filter(|mismatch| *mismatch != Mismatch::DataToml)
                .collect();
            if foreign.is_empty() {
                (lock, local)
            } else {
                let unpushed = lock
                    .packages
                    .values()
                    .any(|entry| !entry.commits.is_empty());
                let mut refused = unpushed;
                for entry in contract.entries() {
                    refused = refused || holds_files(storage, &entry.folder).await;
                }
                if refused {
                    return Err(ReadAllError::Mismatch(foreign));
                }
                // A fresh checkout, filled here: the lock becomes this one's.
                lock.header.filled = None;
                (lock, writer.clone())
            }
        }
        None => (Lock::new(&contract, writer), writer.clone()),
    };

    let mut entries = Vec::new();
    let mut moved = false;
    for entry in contract.entries() {
        if entry.access != Access::Read {
            continue;
        }
        let entry_key = entry.key();
        let locked = lock.packages.get(&entry_key);
        let outcome = if locked.is_some_and(|locked| !locked.commits.is_empty()) {
            EntryRead::Unpushed
        } else {
            let base = locked.map(|locked| locked.base.as_str());
            let mut entry = entry.clone();
            if entry.revision == RevisionPointer::Tag(Tag::Latest)
                && let Some(base) = base
                && !holds_files(storage, &entry.folder).await
            {
                entry.revision = RevisionPointer::Hash(base.to_string());
            }
            match read(storage, remote, &entry, base).await {
                Ok(outcome) => EntryRead::Ran(outcome),
                Err(err) => EntryRead::Failed(err),
            }
        };
        if let EntryRead::Ran(outcome) = &outcome
            && let Some(base) = outcome.lock_base()
        {
            let entry = LockEntry {
                base: base.to_string(),
                commits: Vec::new(),
            };
            lock.packages.insert(entry_key, entry);
            moved = true;
        }
        entries.push((entry.clone(), outcome));
    }

    let read = DataspaceRead { entries };
    if moved {
        // `sync` entries need a Domain, so a domainless read never fills them.
        if lock.header.filled.is_none()
            && read.completed()
            && !contract
                .entries()
                .iter()
                .any(|entry| entry.access == Access::Sync)
        {
            lock.header.filled = Some(Utc::now().trunc_subsecs(0));
        }
        lock.write(storage, &contract, &writer).await?;
    }
    Ok(read)
}

/// Whether `folder` holds anything. One that cannot be listed, for any reason
/// but that it is not there, counts as holding files.
async fn holds_files(storage: &(impl Storage + Sync), folder: &Path) -> bool {
    match storage.read_dir(folder).await {
        Ok(mut listing) => !matches!(listing.next_entry().await, Ok(None)),
        Err(err) => !err.is_not_found(),
    }
}

#[cfg(test)]
mod tests;
