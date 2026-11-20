//! Every `read` entry of a dataspace, read into its working folder with no
//! Domain but, where one is given, its local registry, and the lock written
//! after.

use std::path::Path;

use thiserror::Error;
use tracing::warn;

use crate::dataspace::EntryError;
use crate::dataspace::conflict::read_resolving;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Registry;
use crate::dataspace::hooks;
use crate::dataspace::hooks::HookPoint;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read::ReadReport;
use crate::dataspace::store::name_local;
use crate::domain::LocalRegistry;
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
    /// The working folder was missing or empty, so it was filled whole at
    /// the revision the lock names, not the contract's.
    Refilled(ReadReport),
    /// The lock names local commits of the package that are not pushed. They
    /// are in a Domain, so the package was not read.
    Unpushed,
    /// The read stopped; the lock's entry for the package was left.
    Failed(EntryError),
}

impl EntryRead {
    /// The base the lock took for the package, where its read completed.
    #[must_use]
    pub fn lock_base(&self) -> Option<&str> {
        match self {
            Self::Ran(outcome) => outcome.lock_base(),
            Self::Refilled(report) => Some(&report.revision),
            Self::Unpushed | Self::Failed(_) => None,
        }
    }

    #[must_use]
    pub fn completed(&self) -> bool {
        self.lock_base().is_some()
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
/// lock names, unless the contract names one or that revision can no longer
/// be read whole; the next read moves it on.
///
/// The lock is then written where any read completed: each such package takes
/// the revision as its base, and `filled` is set once every entry that should
/// be read has been.
///
/// A lock written for another `data.toml` is used: the contract was edited
/// here, though [`Lock::write`] leaves an edit under `sync` entries for the
/// sync tick to take. One written for another folder, host or user is used
/// only while no entry's working folder holds files and it names no unpushed commits;
/// otherwise nothing is read or written. The local registry a lock names is
/// not compared, and is kept unless the lock is another's.
///
/// An entry of the local registry `local` is read from it, under its name or
/// one it had, as a bucket's entry is from the bucket, and its key is
/// written under the registry's current name. With no local registry given,
/// a local registry's entry is left unread, and the dataspace unfilled; one
/// naming another local registry fails.
///
/// The contract's `pre-read` hook runs before each entry is read, and one
/// that fails leaves the entry unread. Once the lock is written, `post-read`
/// runs for each package the lock moved to a newer revision.
pub async fn read_all(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    dataspace: &Path,
    writer: &Writer,
) -> Result<DataspaceRead, ReadAllError> {
    read_some(storage, remote, local, dataspace, writer, |_| true).await
}

/// [`read_all`], reading only the `read` entries `select` picks. `filled` is
/// set only once every `read` entry is in the lock.
pub(crate) async fn read_some(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    dataspace: &Path,
    writer: &Writer,
    select: impl Fn(&Entry) -> bool,
) -> Result<DataspaceRead, ReadAllError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(ReadAllError::Mismatch)?;
    name_local(storage, local, &mut contract, &mut lock, &mut writer).await?;
    let unreached = |entry: &Entry| local.is_none() && needs_domain(entry);

    let mut entries = Vec::new();
    let mut newer = Vec::new();
    for entry in contract.entries() {
        if entry.access != Access::Read || unreached(entry) || !select(entry) {
            continue;
        }
        let entry_key = entry.key();
        let locked = lock.packages.get(&entry_key);
        let outcome = if locked.is_some_and(|locked| !locked.commits.is_empty()) {
            EntryRead::Unpushed
        } else if let Err(err) = hooks::run(&contract, HookPoint::PreRead, entry, None).await {
            EntryRead::Failed(err.into())
        } else {
            let base = locked.and_then(|locked| locked.base.as_deref());
            let read = read_entry(storage, remote, local, contract.dataspace(), entry, base).await;
            if let Some(revision) = hooks::newer(base, read.lock_base()) {
                newer.push((entry, revision.to_string()));
            }
            read
        };
        if let Some(base) = outcome.lock_base() {
            lock.packages.insert(entry_key, LockEntry::at(base));
        }
        entries.push((entry.clone(), outcome));
    }

    let read = DataspaceRead { entries };
    if read.entries.iter().any(|(_, read)| read.completed()) {
        // An entry left unselected is unfilled until the lock holds it.
        let unfilled = contract.entries().iter().any(|entry| {
            entry.access == Access::Sync
                || (entry.access == Access::Read
                    && (unreached(entry) || !lock.packages.contains_key(&entry.key())))
        });
        // `sync` entries need a Domain, and local packages their registry,
        // so a domainless read never fills them.
        if lock.header.filled.is_none() && read.completed() && !unfilled {
            lock.header.filled = Some(crate::time::to_second(jiff::Timestamp::now()));
        }
        lock.write(storage, &contract, &writer).await?;
    }
    for (entry, revision) in newer {
        hooks::after(&contract, HookPoint::PostRead, entry, &revision).await;
    }
    Ok(read)
}

/// The lock beside `contract`, or a new one, and the writer to write it as;
/// or what in a lock written for another folder, host or user keeps it from
/// use. See [`read_all`].
pub(crate) async fn usable_lock(
    storage: &(impl Storage + Sync),
    contract: &Contract,
    writer: &Writer,
) -> Result<Result<(Lock, Writer), Vec<Mismatch>>, LockError> {
    let Some(mut lock) = Lock::read(storage, contract).await? else {
        return Ok(Ok((Lock::new(contract, writer), writer.clone())));
    };
    let local = Writer {
        local_registry: lock.header.writer.local_registry.clone(),
        ..writer.clone()
    };
    let foreign: Vec<Mismatch> = lock
        .mismatches(contract, &local)
        .into_iter()
        .filter(|mismatch| *mismatch != Mismatch::DataToml)
        .collect();
    if foreign.is_empty() {
        return Ok(Ok((lock, local)));
    }
    let mut refused = lock
        .packages
        .values()
        .any(|entry| !entry.commits.is_empty());
    for entry in contract.entries() {
        refused = refused || holds_files(storage, &entry.folder).await;
    }
    if refused {
        return Ok(Err(foreign));
    }
    // A fresh checkout, filled here: the lock becomes this one's.
    lock.header.filled = None;
    Ok(Ok((lock, writer.clone())))
}

/// Reads `entry` against `base`, as [`read_resolving`] does in `dataspace`; a
/// working folder that is missing or empty first at `base`, unless the entry
/// names a revision.
async fn read_entry(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    dataspace: &Path,
    entry: &Entry,
    base: Option<&str>,
) -> EntryRead {
    if entry.revision == RevisionPointer::Tag(Tag::Latest)
        && let Some(locked) = base
        && !holds_files(storage, &entry.folder).await
    {
        let mut refill = entry.clone();
        refill.revision = RevisionPointer::Hash(locked.to_string());
        // A revision that can no longer be read whole gives way to the
        // contract's.
        let why = match read_resolving(storage, remote, local, dataspace, &refill, base).await {
            Ok(ReadOutcome::Read(report))
                if report.skipped.is_empty() && report.hash_mismatch.is_empty() =>
            {
                return EntryRead::Refilled(report);
            }
            Ok(ReadOutcome::Read(_)) => "some of its files are gone".to_string(),
            Ok(outcome) => return EntryRead::Ran(outcome),
            // The bucket no longer holds it; any other failure is reported.
            Err(EntryError::Failed(err)) if matches!(&*err, crate::Error::S3(s3) if s3.is_not_found()) => {
                err.to_string()
            }
            Err(err) => return EntryRead::Failed(err),
        };
        warn!(
            "{} was not refilled at {locked}, so it takes the contract's revision: {why}",
            entry.folder.display()
        );
    }
    match read_resolving(storage, remote, local, dataspace, entry, base).await {
        Ok(outcome) => EntryRead::Ran(outcome),
        Err(err) => EntryRead::Failed(err),
    }
}

/// A local registry's package, which only the Domain holds.
fn needs_domain(entry: &Entry) -> bool {
    matches!(entry.registry, Registry::Local(_))
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
