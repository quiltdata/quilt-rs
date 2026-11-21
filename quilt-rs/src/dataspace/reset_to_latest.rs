//! `reset_to_latest`: a package's local commits and local changes discarded,
//! and `latest` read whole into its working folder as its base. The commits
//! stay in the Domain's store, where
//! [`reattach`](crate::dataspace::reattach::reattach) reaches them.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use tracing::info;
use tracing::warn;

use crate::LogicalKey;
use crate::dataspace::EntryError;
use crate::dataspace::conflict::PackageRows;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::entry_under;
use crate::dataspace::hooks;
use crate::dataspace::hooks::HookPoint;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::metadata::render;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::replace::Unfetched;
use crate::dataspace::replace::replace;
use crate::dataspace::store::Store;
use crate::dataspace::store::current_key;
use crate::dataspace::store::name_local;
use crate::dataspace::sync::base_manifest;
use crate::dataspace::sync::covered_rows;
use crate::dataspace::sync::latest_in;
use crate::dataspace::sync::local_commit;
use crate::dataspace::sync::manifest_at;
use crate::dataspace::sync::moved_from;
use crate::domain::DomainRoot;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;

/// What [`reset_to_latest`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Reset {
    /// The revision `latest` named, read whole and now the package's base.
    pub revision: String,
    /// The local commits the lock named before, oldest first, which the
    /// Domain's store keeps.
    pub discarded: Vec<String>,
    /// The files the revision put in the folder, by logical key.
    pub placed: Vec<PathBuf>,
    /// The files of the package's revision before that the revision lacks,
    /// removed, by logical key.
    pub removed: Vec<PathBuf>,
}

/// Why nothing was reset.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ResetError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to reset",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{CONTRACT_FILE} does not declare {0}")]
    NotDeclared(Box<PackageKey>),

    /// A `write` or `replace` entry is not read, so it has nothing to reset.
    #[error("the package is under `{0}`, which is not read, so there is nothing to reset")]
    Access(Access),

    #[error("the entry is pinned to {0}, so it does not follow `latest`")]
    Pinned(String),

    #[error("the package has no `latest` to reset to")]
    NoLatest,

    #[error(
        "{} files of `latest` could not be fetched whole, so nothing was changed",
        .0.len()
    )]
    Unfetched(Vec<PathBuf>),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for ResetError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for ResetError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for ResetError {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Resets `package` in the dataspace in the folder `dataspace` to the
/// revision `latest` names: its local commits are dropped from the lock and
/// its local changes discarded, the revision is read whole into the working
/// folder, and the lock, written last, takes it as the package's base. It
/// is an explicit act of the user's, so nothing in the folder blocks it.
///
/// Each file of the package's current revision, and of `latest`, under the
/// entry's paths is replaced by `latest`'s, or removed where `latest` lacks
/// it, and `metadata.toml` is written from `latest`. A file neither
/// revision holds is the user's own, never committed, and is left, as
/// today's reset leaves an untracked file. Every file of `latest` is fetched
/// before the folder is touched, and the files replaced are kept aside until
/// the last is placed: a reset that fails part-way puts them back, and
/// leaves the lock as it was. The commits dropped stay in `domain`'s store,
/// where [`reattach`](crate::dataspace::reattach) lists them. The package's
/// rows in `conflict.json` are removed. A current revision that can no
/// longer be found, as one removed from its registry, is taken as empty.
/// A package `data.toml` moved from this machine's local registry to its
/// bucket, whose move has not landed, is reset to the bucket's `latest`:
/// the move is dropped with its local commits, which stay in the local
/// registry.
///
/// A `read` or `sync` entry is reset, one pinned to a revision is refused,
/// and so is a package with no `latest`. The contract's `pre-read` hook runs
/// first, and `post-read` once the lock moved to another revision. The
/// lock is used as [`read_all`](crate::dataspace::read_all::read_all) uses
/// it.
pub async fn reset_to_latest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Reset, ResetError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(ResetError::Mismatch)?;
    let local = domain.local();
    name_local(storage, Some(&local), &mut contract, &mut lock, &mut writer).await?;
    let key = current_key(storage, Some(&local), package).await?;
    let entry = read_entry(&contract, &key)?;
    let pin = match &entry.revision {
        RevisionPointer::Tag(Tag::Latest) => None,
        RevisionPointer::Hash(hash) | RevisionPointer::HashPrefix(hash) => Some(hash.clone()),
        RevisionPointer::Tag(tag) => Some(format!(":{tag}")),
    };
    if let Some(pin) = pin {
        return Err(ResetError::Pinned(pin));
    }
    let store = Store::of(storage, domain, &entry).await?;
    let Some(_held) = storage
        .try_lock_exclusive(store.package_lock(&entry.package))
        .await?
    else {
        return Err(ResetError::Busy(Box::new(key)));
    };
    let latest = latest_in(storage, remote, store.local(), &entry)
        .await?
        .ok_or(ResetError::NoLatest)?;

    // A move to the bucket that has not landed is dropped with its local
    // commits, which stay in the local registry for reattach.
    let base = lock
        .packages
        .get(&key)
        .and_then(|locked| locked.base.clone());
    let moved = moved_from(&lock, &contract, &writer, &entry);
    let (current, discarded) = if let Some((local_key, head)) = &moved {
        let current = match local_commit(storage, &local, &entry, head).await {
            Ok(current) => current,
            Err(EntryError::Failed(err)) if err.is_not_found() => {
                warn!("The revision {head} the move names is gone, so it is taken as empty");
                Manifest::default()
            }
            Err(err) => return Err(err.into()),
        };
        let discarded = lock
            .packages
            .get(local_key)
            .map_or_else(Vec::new, |locked| locked.commits.clone());
        (current, discarded)
    } else {
        let discarded = lock
            .packages
            .get(&key)
            .map_or_else(Vec::new, |locked| locked.commits.clone());
        let current = match discarded.last().or(base.as_ref()) {
            None => Manifest::default(),
            Some(hash) => {
                match manifest_at(storage, remote, &store, &entry, hash, &discarded).await {
                    Ok(current) => current,
                    Err(EntryError::Failed(err)) if err.is_not_found() => {
                        warn!(
                            "The revision {hash} the lock names is gone, so it is taken as empty"
                        );
                        Manifest::default()
                    }
                    Err(err) => return Err(err.into()),
                }
            }
        };
        (current, discarded)
    };

    hooks::run(&contract, HookPoint::PreRead, &entry, None)
        .await
        .map_err(EntryError::from)?;
    let target = base_manifest(storage, remote, &store, &entry, &latest).await?;
    let (placed, removed) = Box::pin(replace_files(
        storage, remote, &store, &entry, &latest, &current, &target,
    ))
    .await?;

    let rows = PackageRows::of(storage, store.local(), contract.dataspace(), &entry).await;
    if let Err(err) = rows.write(storage, None).await {
        warn!("The package's rows were left in conflict.json: {err}");
    }
    if let Some((local_key, _)) = &moved {
        lock.packages.remove(local_key);
    }
    lock.packages
        .insert(key.clone(), LockEntry::at(latest.clone()));
    lock.write(storage, &contract, &writer).await?;
    if let Some(revision) = hooks::newer(base.as_deref(), Some(&latest)) {
        hooks::after(&contract, HookPoint::PostRead, &entry, revision).await;
    }
    info!("✔️ Reset {key} to {latest}");
    Ok(Reset {
        revision: latest,
        discarded,
        placed,
        removed,
    })
}

/// `key`'s entry that is read in `contract`: under `sync`, or `read`.
fn read_entry(contract: &Contract, key: &PackageKey) -> Result<Entry, ResetError> {
    entry_under(contract, key, Access::Sync)
        .or_else(|_| entry_under(contract, key, Access::Read))
        .map_err(|other| match other {
            Some(access) => ResetError::Access(access),
            None => ResetError::NotDeclared(Box::new(key.clone())),
        })
}

/// Replaces the files of `current` and of `target`, the revision `hash`,
/// under `entry`'s paths in its working folder with `target`'s, and writes
/// its `metadata.toml`; see [`reset_to_latest`]. Returns the keys placed and
/// those removed.
async fn replace_files(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    hash: &str,
    current: &Manifest,
    target: &Manifest,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), ResetError> {
    let text = render(target).map_err(EntryError::MetadataUnwritable)?;
    let wanted: BTreeSet<&LogicalKey> = covered_rows(entry, target)
        .into_iter()
        .map(|row| &row.logical_key)
        .collect();
    let had: BTreeSet<&LogicalKey> = covered_rows(entry, current)
        .into_iter()
        .map(|row| &row.logical_key)
        .collect();
    if let Err(Unfetched(unfetched)) = replace(
        storage,
        remote,
        store,
        entry,
        (hash, target),
        &wanted,
        &had,
        Some(text),
    )
    .await?
    {
        return Err(ResetError::Unfetched(unfetched));
    }
    let removed = had
        .difference(&wanted)
        .map(|key| key.to_path_buf())
        .collect();
    let placed = wanted.iter().map(|key| key.to_path_buf()).collect();
    Ok((placed, removed))
}

#[cfg(test)]
mod tests;
