//! `undo_commit`: a package's last local commit dropped from the lock and
//! its working folder restored to the revision before it, as
//! [`flow::undo_commit`](crate::flow::undo_commit) restores a ref's. The
//! commit stays in the Domain's store, where
//! [`reattach`](crate::dataspace::reattach::reattach) reaches it.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use tracing::info;

use crate::LogicalKey;
use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::metadata::render;
use crate::dataspace::read::OnDisk;
use crate::dataspace::read::bytes_at;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::replace::Unfetched;
use crate::dataspace::replace::replace;
use crate::dataspace::store::Store;
use crate::dataspace::store::current_key;
use crate::dataspace::store::name_local;
use crate::dataspace::sync::covered_rows;
use crate::dataspace::sync::manifest_at;
use crate::dataspace::sync::pending_move;
use crate::domain::DomainRoot;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;

/// What [`undo_commit`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Undone {
    /// The commit dropped from the lock, which the Domain's store keeps.
    pub commit: String,
    /// The package's lock entry after it; `None` where the commit was the
    /// only one of a package never published, whose entry went with it.
    pub locked: Option<LockEntry>,
    /// The files the folder took from the revision restored, by logical
    /// key.
    pub restored: Vec<PathBuf>,
    /// The files the undone commit added, removed, by logical key.
    pub removed: Vec<PathBuf>,
}

/// Why nothing was undone.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UndoError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to undo a commit",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    /// The lock names no unpushed commit of the package. A push releases the
    /// commits it squashed, so undo never steps back past one (25.9).
    #[error(
        "{LOCK_FILE} names no local commit of {0} that was not pushed, and undo does not step back past a push"
    )]
    NothingToUndo(Box<PackageKey>),

    #[error("{CONTRACT_FILE} does not declare {0}")]
    NotDeclared(Box<PackageKey>),

    /// A path the undo would write over does not hold what the undone
    /// commit put there, so it holds work not committed.
    #[error(
        "{} would be overwritten, and does not hold what the commit put there; commit or restore it, then undo",
        .0.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    Uncommitted(Vec<PathBuf>),

    #[error(
        "{} files of the revision before could not be fetched whole, so nothing was changed",
        .0.len()
    )]
    Unfetched(Vec<PathBuf>),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for UndoError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for UndoError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for UndoError {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Undoes `package`'s last local commit in the dataspace in the folder
/// `dataspace`: its working folder is restored to the revision before it,
/// the commit before, or else the base, or else nothing for a package never
/// published, and the commit is dropped from the lock, written last.
///
/// As [`flow::undo_commit`](crate::flow::undo_commit) does, the folder
/// takes the manifest's change, not a rebuild: under the entry's paths, a
/// file the two revisions agree on is left alone, one the commit changed
/// or deleted takes the revision before's bytes, and one it added is
/// removed, with the folders that leaves empty. `metadata.toml` takes the
/// revision before's metadata, or goes with nothing to restore. A file
/// neither revision holds is not touched. Nothing is written over: unless
/// every path the undo would touch holds what the commit put there, or
/// nothing where it put nothing, the undo is refused, so work not
/// committed is never lost. The files are fetched before the folder
/// changes, from the store or the bucket, and an undo that fails part-way
/// puts the folder back.
///
/// The commit stays in `domain`'s store, where
/// [`reattach`](crate::dataspace::reattach) lists it once no lock names it.
/// The lock names only commits not pushed: a push releases the ones it
/// squashed, so undo cannot step back past it (25.9), and a package with no
/// commit in the lock is refused. A bucket's package and a local
/// registry's are undone alike, and a package `data.toml` moved from the
/// local registry to its bucket, whose move has not landed, is undone in
/// the local registry. The package's lock in the Domain is held
/// meanwhile, so a tick does not push the commit underneath. The lock is
/// used as [`read_all`](crate::dataspace::read_all::read_all) uses it.
pub async fn undo_commit(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Undone, UndoError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(UndoError::Mismatch)?;
    let local = domain.local();
    name_local(storage, Some(&local), &mut contract, &mut lock, &mut writer).await?;
    let key = current_key(storage, Some(&local), package).await?;
    let entry = contract
        .entries()
        .iter()
        .find(|entry| entry.key() == key)
        .cloned()
        .ok_or_else(|| UndoError::NotDeclared(Box::new(key.clone())))?;
    let entry = pending_move(&lock, &contract, &writer, entry);
    let key = entry.key();
    let store = Store::of(storage, domain, &entry).await?;
    let Some(_held) = storage
        .try_lock_exclusive(store.package_lock(&entry.package))
        .await?
    else {
        return Err(UndoError::Busy(Box::new(key)));
    };

    let Some(locked) = lock.packages.get(&key).cloned() else {
        return Err(UndoError::NothingToUndo(Box::new(key)));
    };
    let Some((commit, before)) = locked.commits.split_last() else {
        return Err(UndoError::NothingToUndo(Box::new(key)));
    };
    let current = manifest_at(storage, remote, &store, &entry, commit, &locked.commits).await?;
    let previous_hash = before.last().or(locked.base.as_ref());
    let previous = match previous_hash {
        Some(hash) => manifest_at(storage, remote, &store, &entry, hash, before).await?,
        None => Manifest::default(),
    };

    let (restored, removed) = restore(
        storage,
        remote,
        &store,
        &entry,
        (
            &current,
            previous_hash.map(|hash| (hash.as_str(), &previous)),
        ),
    )
    .await?;

    let undone = LockEntry {
        base: locked.base.clone(),
        commits: before.to_vec(),
    };
    if undone.current().is_some() {
        lock.packages.insert(key.clone(), undone);
    } else {
        lock.packages.remove(&key);
    }
    let after = lock.packages.get(&key).cloned();
    lock.write(storage, &contract, &writer).await?;
    info!("✔️ Undid commit {commit} of {key}");
    Ok(Undone {
        commit: commit.clone(),
        locked: after,
        restored,
        removed,
    })
}

/// Restores `entry`'s working folder from `current`, the commit undone, to
/// `previous`, the revision before it and its hash, `None` for none: see
/// [`undo_commit`]. Returns the keys restored and those removed.
async fn restore(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    (current, previous): (&Manifest, Option<(&str, &Manifest)>),
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), UndoError> {
    let empty = Manifest::default();
    let (hash, before) = previous.unwrap_or(("", &empty));
    let committed = covered_rows(entry, current);
    let restoring = covered_rows(entry, before);
    // The rows the two revisions agree on stay as they are.
    let place: BTreeSet<&LogicalKey> = restoring
        .iter()
        .filter(|row| {
            !committed
                .iter()
                .any(|had| had.logical_key == row.logical_key && had.hash == row.hash)
        })
        .map(|row| &row.logical_key)
        .collect();
    let added: BTreeSet<&LogicalKey> = committed
        .iter()
        .map(|row| &row.logical_key)
        .filter(|key| !restoring.iter().any(|row| row.logical_key == **key))
        .collect();
    let clear: BTreeSet<&LogicalKey> = place.union(&added).copied().collect();

    // Every path written over holds what the commit put there.
    let folder = &entry.folder;
    let mut uncommitted = Vec::new();
    for key in &clear {
        let path = key.join_to(folder);
        let as_committed = match committed.iter().find(|row| row.logical_key == **key) {
            Some(row) => OnDisk::read(storage, &path, row.hash.algorithm())
                .await?
                .holds(row),
            // A folder is no file of the user's: the files under it are
            // each checked.
            None => !storage.is_file(&path).await && !storage.is_symlink(&path).await,
        };
        if !as_committed {
            uncommitted.push(key.to_path_buf());
        }
    }
    let metadata_file = bytes_at(storage, &folder.join(METADATA_FILE)).await?;
    let committed_text = render(current).map_err(EntryError::MetadataUnwritable)?;
    if metadata_file.is_some_and(|bytes| bytes != committed_text.as_bytes()) {
        uncommitted.push(PathBuf::from(METADATA_FILE));
    }
    if !uncommitted.is_empty() {
        return Err(UndoError::Uncommitted(uncommitted));
    }

    let metadata = match previous {
        Some(_) => Some(render(before).map_err(EntryError::MetadataUnwritable)?),
        None => None,
    };
    if let Err(Unfetched(unfetched)) = replace(
        storage,
        remote,
        store,
        entry,
        (hash, before),
        &place,
        &clear,
        metadata,
    )
    .await?
    {
        return Err(UndoError::Unfetched(unfetched));
    }
    Ok((
        place.iter().map(|key| key.to_path_buf()).collect(),
        added.iter().map(|key| key.to_path_buf()).collect(),
    ))
}

#[cfg(test)]
mod tests;
