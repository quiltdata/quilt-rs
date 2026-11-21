//! `commit`: a `sync` entry's working folder recorded as a local commit with
//! a message, as the tick's commit records it, and appended to the
//! package's commits in the lock. Nothing is pushed: the next tick pushes it.

use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use tracing::info;
use tracing::warn;

use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Paths;
use crate::dataspace::entry_under;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read::gone;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::Store;
use crate::dataspace::store::current_key;
use crate::dataspace::store::name_local;
use crate::dataspace::sync::FolderCheck;
use crate::dataspace::sync::FolderCommit;
use crate::dataspace::sync::append_commit;
use crate::dataspace::sync::commit_folder;
use crate::dataspace::sync::manifest_at;
use crate::dataspace::sync::pending_move;
use crate::dataspace::sync::rewrite_metadata;
use crate::domain::DomainRoot;
use crate::flow::WorkflowWarning;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;

/// What [`commit`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Committed {
    /// The package's `sync` entry, as the contract expanded it.
    pub entry: Entry,
    /// The commit recorded; `None` where the folder held nothing the lock's
    /// current revision lacks, and nothing was written.
    pub commit: Option<String>,
    /// The package's lock entry after the commit.
    pub locked: Option<LockEntry>,
    /// What checking the commit against its bucket's current workflows
    /// config found: the commit is recorded either way. Empty where it
    /// passed, nothing was committed, or the package is in a local registry,
    /// which has no workflows.
    pub warnings: Vec<WorkflowWarning>,
}

/// Why nothing was committed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommitError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to commit",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{CONTRACT_FILE} does not declare {0}")]
    NotDeclared(Box<PackageKey>),

    /// Only a `sync` entry takes a local commit: nothing pushes one under
    /// `write` or `replace`, which publish their folder whole, and `read`
    /// never publishes.
    #[error(
        "the package is under `{0}`, and only a `sync` entry takes a local commit; publish a `write` or `replace` entry instead"
    )]
    Access(Access),

    /// `data.toml` changed since the lock was written, so paths it added may
    /// not be read yet, and an absent file cannot count as deleted.
    #[error(
        "{CONTRACT_FILE} changed since {LOCK_FILE} was written, so a missing file is not yet a deletion; sync the dataspace, then commit"
    )]
    Edited,

    #[error("the working folder {} is not there, so there is nothing to commit", .0.display())]
    NoFolder(PathBuf),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for CommitError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for CommitError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for CommitError {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Commits `package`'s working folder in the dataspace in the folder
/// `dataspace` as a local commit with `message`, on the lock's current
/// revision, and appends it to the package's commits in the lock, written
/// last. A package never published gets an entry with commits and no base
/// (25.6), which its first push bases.
///
/// The folder is taken as the tick's commit takes it: under the entry's
/// paths, with the contract's ignore policy and the user-wide `user_rules`
/// file, a file the folder lacks counted as deleted, and `metadata.toml`
/// taken as complete within the entry's paths. `message` wins over the
/// file's. The contract's `pre-write` hook runs first where the folder
/// changed, and the folder is then taken again; `post-write` waits for the
/// push. The commit takes the bucket's default workflow, and is checked
/// against it only when it is pushed; a local registry's takes none. A
/// commit that is the base again releases the commits before it, as the
/// tick does. The folder's `metadata.toml` is then rewritten from the commit.
///
/// Only a `sync` entry takes a commit, since it needs a Domain (25.12): the
/// commit is kept in `domain`'s store for the bucket, or in its local
/// registry. A revision the lock names was read whole into the folder
/// before the lock took it (25.7 rule 2), so the dataspace need not be
/// filled; but after `data.toml` was edited, a package with a revision in
/// the lock is refused until a sync reads it again, since paths the edit
/// added are not read yet (rule 4). So is a folder that is missing or empty
/// while the lock names a revision. The lock is used as
/// [`read_all`](crate::dataspace::read_all::read_all) uses it.
///
/// A package `data.toml` moved from this machine's local registry to its
/// bucket, whose move has not landed, is committed in the local registry,
/// after its last local commit, so the move pushes the new commit. A whole
/// package's move is not refused for the edit that made it.
#[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
pub async fn commit(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
    message: &str,
    user_rules: &Path,
) -> Result<Committed, CommitError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(CommitError::Mismatch)?;
    let local = domain.local();
    name_local(storage, Some(&local), &mut contract, &mut lock, &mut writer).await?;
    let key = current_key(storage, Some(&local), package).await?;
    let entry = entry_under(&contract, &key, Access::Sync).map_err(|other| match other {
        Some(access) => CommitError::Access(access),
        None => CommitError::NotDeclared(Box::new(key.clone())),
    })?;
    let declared = entry.clone();
    let entry = pending_move(&lock, &contract, &writer, entry);
    let key = entry.key();
    // The edit that moved a whole package is what keeps its move pending,
    // and no sync takes it until the move lands; it added no path to read.
    let moving_whole = entry.registry != declared.registry && entry.paths == Paths::Whole;
    let store = Store::of(storage, domain, &entry).await?;
    // The lock a tick moving the package takes: the bucket's.
    let lock_store = Store::of(storage, domain, &declared).await?;
    let Some(_held) = storage
        .try_lock_exclusive(lock_store.package_lock(&declared.package))
        .await?
    else {
        return Err(CommitError::Busy(Box::new(key)));
    };

    let locked = lock.packages.get(&key).cloned();
    let (base, commits) = locked
        .clone()
        .map_or((None, Vec::new()), |locked| (locked.base, locked.commits));
    let current_hash = commits.last().or(base.as_ref()).cloned();
    // A base in the lock was read whole first (25.7 rule 2), but paths an
    // edit to `data.toml` added were not yet read (rule 4).
    let edited = lock.header.data_toml_hash != contract.data_toml_hash();
    if current_hash.is_some() && edited && !moving_whole {
        return Err(CommitError::Edited);
    }
    let folder = &entry.folder;
    let current = match &current_hash {
        Some(hash) => {
            let current = manifest_at(storage, remote, &store, &entry, hash, &commits).await?;
            if gone(storage, folder, &current, &entry.paths).await? {
                return Err(EntryError::FolderGone(folder.clone()).into());
            }
            current
        }
        None if !storage.exists(folder).await => {
            return Err(CommitError::NoFolder(folder.clone()));
        }
        None => Manifest::default(),
    };

    let committed = commit_folder(
        storage,
        remote,
        &store,
        &contract,
        &entry,
        user_rules,
        (current_hash.as_deref(), &current),
        message,
        FolderCheck::Warn,
    )
    .await?;
    let Some(FolderCommit {
        hash,
        metadata_text,
        warnings,
    }) = committed
    else {
        return Ok(Committed {
            entry,
            commit: None,
            locked,
            warnings: Vec::new(),
        });
    };
    if base.as_deref() == Some(hash.as_str()) {
        // Committed back to the base: the commits before it are released.
        lock.packages
            .insert(key.clone(), LockEntry::at(hash.clone()));
    } else {
        append_commit(&mut lock, &key, &hash);
    }
    lock.write(storage, &contract, &writer).await?;
    if let Err(err) =
        rewrite_metadata(storage, &store, &entry, &hash, metadata_text.as_deref()).await
    {
        warn!("{METADATA_FILE} was left as it is: {err}");
    }
    info!("✔️ Committed {key} as {hash}");
    for warning in &warnings {
        warn!("{warning}");
    }
    Ok(Committed {
        entry,
        commit: Some(hash),
        locked: lock.packages.get(&key).cloned(),
        warnings,
    })
}

#[cfg(test)]
mod tests;
