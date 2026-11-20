//! `certify_latest`: a package's `latest` forced to its current revision,
//! whatever it names. It is today's Promote, the separate and explicit act
//! 25.9 keeps apart from `replace`, and how a revision the workflow refused,
//! or a move to a bucket it refused, is released (25k decision 1).

use std::path::Path;

use thiserror::Error;
use tracing::info;

use crate::dataspace::EntryError;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::entry_under;
use crate::dataspace::hooks;
use crate::dataspace::hooks::HookPoint;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::publish::Published;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::Store;
use crate::dataspace::store::current_key;
use crate::dataspace::store::name_local;
use crate::dataspace::sync::folder_key;
use crate::dataspace::sync::latest as read_latest;
use crate::dataspace::sync::local_commit;
use crate::dataspace::sync::manifest_at;
use crate::dataspace::sync::moved_from;
use crate::dataspace::sync::push_manifest;
use crate::dataspace::sync::revision;
use crate::domain::DomainRoot;
use crate::io::manifest::tag_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::queue::Queue;

/// What [`certify_latest`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Certified {
    /// The revision `latest` names now, and the lock's base for the package.
    pub revision: String,
    /// What `latest` named before; `None` where the package had none.
    pub previous: Option<String>,
    /// How the push of a local commit ended, before `latest` was forced to
    /// it; `None` where the revision was in its registry already.
    pub pushed: Option<Published>,
}

/// Why `latest` was not forced.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CertifyError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to certify",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{CONTRACT_FILE} does not declare {0}")]
    NotDeclared(Box<PackageKey>),

    /// Only a `sync` entry's current revision is the lock's to certify.
    #[error("the package is under `{0}`, and only a `sync` entry's revision is certified")]
    Access(Access),

    #[error("{LOCK_FILE} names no revision of {0} to certify")]
    NothingToCertify(Box<PackageKey>),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for CertifyError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for CertifyError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for CertifyError {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Forces `package`'s `latest` to its current revision in the dataspace in
/// the folder `dataspace`: the lock's last commit, or else its base. A
/// commit is pushed first, against the lock's base, and `latest` is then
/// written whatever it names, unchecked, as today's Promote writes it: the
/// last writer wins. A revision the bucket's workflow refuses is stored and
/// made `latest` all the same, which is how a blocked revision is released;
/// one stored and kept in `queue` is not uploaded again, and is released
/// there. A push that cannot tell where `latest` went fails rather than
/// forcing it (25c reading 6). The lock, written last, takes the revision
/// as the package's base, its commits released, as after any push.
///
/// A package `data.toml` moved from this machine's local registry to its
/// bucket, whose move the tick could not finish, is moved here: its last
/// local commit is pushed and forced, and the lock then holds it under the
/// bucket's key. A package of `domain`'s local registry is forced there,
/// under its lock. The contract's `post-write` hook runs once something was
/// stored or `latest` moved.
///
/// Only a `sync` entry is certified: a `read` entry never writes, and a
/// `write` or `replace` entry keeps no revision of its own in the lock. The
/// lock is used as [`read_all`](crate::dataspace::read_all::read_all) uses
/// it.
pub async fn certify_latest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    queue: &Queue,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Certified, CertifyError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(CertifyError::Mismatch)?;
    let local = domain.local();
    name_local(storage, Some(&local), &mut contract, &mut lock, &mut writer).await?;
    let key = current_key(storage, Some(&local), package).await?;
    let entry = entry_under(&contract, &key, Access::Sync).map_err(|other| match other {
        Some(access) => CertifyError::Access(access),
        None => CertifyError::NotDeclared(Box::new(key.clone())),
    })?;
    let store = Store::of(storage, domain, &entry).await?;
    let Some(_held) = storage
        .try_lock_exclusive(store.package_lock(&entry.package))
        .await?
    else {
        return Err(CertifyError::Busy(Box::new(key)));
    };

    let moved = moved_from(&lock, &contract, &writer, &entry);
    let (head, base, commits) = match (lock.packages.get(&key), &moved) {
        (Some(locked), _) => {
            let head = locked
                .current()
                .ok_or_else(|| CertifyError::NothingToCertify(Box::new(key.clone())))?;
            (
                head.to_string(),
                locked.base.clone(),
                locked.commits.clone(),
            )
        }
        (None, Some((_, head))) => (head.clone(), None, vec![head.clone()]),
        (None, None) => return Err(CertifyError::NothingToCertify(Box::new(key))),
    };

    let (previous, pushed, wrote) = match &store {
        Store::Local(registry) => {
            let previous = registry
                .force_latest_held(storage, &entry.package, &head)
                .await?;
            let wrote = previous.as_deref() != Some(head.as_str());
            (previous, None, wrote)
        }
        Store::Bucket(_) if commits.is_empty() => {
            let previous = read_latest(remote, &entry).await?;
            let wrote = previous.as_deref() != Some(head.as_str());
            if wrote {
                tag_latest(remote, &revision(&entry, &head)?).await?;
            }
            (previous, None, wrote)
        }
        Store::Bucket(_) => {
            let manifest = if moved.is_some() {
                local_commit(storage, &local, &entry, &head).await?
            } else {
                manifest_at(storage, remote, &store, &entry, &head, &commits).await?
            };
            let (pushed, stored) = Box::pin(push_manifest(
                storage,
                remote,
                queue,
                &entry,
                &head,
                manifest,
                base.as_deref(),
            ))
            .await?;
            let previous = match &pushed {
                Published::Advanced { .. } => base.clone(),
                Published::Stored { latest, .. } => latest.clone(),
                _ => read_latest(remote, &entry).await?,
            };
            let forced = !matches!(pushed, Published::Advanced { .. });
            if forced {
                tag_latest(remote, &revision(&entry, &head)?).await?;
            }
            (previous, Some(pushed), stored || forced)
        }
    };

    let folder = folder_key(&entry);
    let kept = queue.kept(&folder).await.map_err(EntryError::from)?;
    if kept.is_some_and(|kept| kept.hash == head) {
        queue.release(&folder).await.map_err(EntryError::from)?;
    }
    if let Some((local_key, _)) = &moved {
        lock.packages.remove(local_key);
    }
    lock.packages
        .insert(key.clone(), LockEntry::at(head.clone()));
    lock.write(storage, &contract, &writer).await?;
    if wrote {
        hooks::after(&contract, HookPoint::PostWrite, &entry, &head).await;
    }
    info!("✔️ Certified {head} as the latest revision of {key}");
    Ok(Certified {
        revision: head,
        previous,
        pushed,
    })
}

#[cfg(test)]
mod tests;
