//! Every `write` and `replace` entry of a dataspace, published with no
//! Domain, and the lock written after.

use std::path::Path;

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
use crate::dataspace::publish::Base;
use crate::dataspace::publish::Published;
use crate::dataspace::publish::publish;
use crate::dataspace::read::read;
use crate::dataspace::read_all::EntryRead;
use crate::io::remote::Remote;
use crate::io::storage::Storage;

/// How one `write` or `replace` entry's publish ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum EntryPublish {
    /// The publish ran. The lock took its revision where
    /// [`Published::lock_base`] names one.
    Ran(Published),
    /// The package is also under `read`, and its read did not complete, so
    /// nothing was published.
    NotRead,
    /// The lock names local commits of the package that are not pushed. They
    /// are in a Domain, so the package was not published.
    Unpushed,
    /// The publish stopped; the lock's entry for the package was left.
    Failed(EntryError),
}

impl EntryPublish {
    /// Whether `latest` holds the folder's files: the publish advanced it, or
    /// found nothing to publish.
    #[must_use]
    pub fn completed(&self) -> bool {
        matches!(
            self,
            Self::Ran(Published::Advanced { .. } | Published::Unchanged { .. })
        )
    }
}

/// One `write` or `replace` entry, and how its publish ended.
#[derive(Debug)]
#[non_exhaustive]
pub struct PackagePublish {
    pub entry: Entry,
    /// The read of the package's `read` entry, run first because the base
    /// was not `latest`.
    pub read: Option<EntryRead>,
    pub published: EntryPublish,
}

/// What [`publish_all`] did, entry by entry.
#[derive(Debug)]
pub struct DataspacePublish {
    /// Each `write` and `replace` entry, in the contract's order.
    pub entries: Vec<PackagePublish>,
}

impl DataspacePublish {
    #[must_use]
    pub fn completed(&self) -> bool {
        self.entries.iter().all(|entry| entry.published.completed())
    }
}

/// Why nothing of a dataspace was published.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PublishAllError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; fix or remove it first",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),
}

impl From<ContractError> for PublishAllError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for PublishAllError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

/// Publishes each `write` and `replace` entry of the dataspace in the folder
/// `dataspace` from its working folder, leaving out what the ignore rules
/// leave out of a commit, `user_rules` being the user-wide file.
///
/// A package also under `read` has one base, the lock's: it is published
/// only while that base is `latest`, or with no base while the package has no
/// revision. Where it is not, or the lock was written for another `data.toml`,
/// the `read` entry is read first and the publish is on the revision read.
///
/// The lock is then written where any base moved. A lock written for another
/// `data.toml` is used: the contract was edited here. One written for another
/// folder, host or user is refused, and nothing is published.
pub async fn publish_all(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    dataspace: &Path,
    writer: &Writer,
    user_rules: &Path,
) -> Result<DataspacePublish, PublishAllError> {
    let contract = Contract::read(storage, dataspace).await?;
    let (mut lock, writer) = match Lock::read(storage, &contract).await? {
        Some(lock) => {
            let writer = Writer {
                local_registry: lock.header.writer.local_registry.clone(),
                ..writer.clone()
            };
            let foreign: Vec<Mismatch> = lock
                .mismatches(&contract, &writer)
                .into_iter()
                .filter(|mismatch| *mismatch != Mismatch::DataToml)
                .collect();
            if !foreign.is_empty() {
                return Err(PublishAllError::Mismatch(foreign));
            }
            (lock, writer)
        }
        None => (Lock::new(&contract, writer), writer.clone()),
    };
    // A base recorded before `data.toml` was edited may predate the read
    // paths, which are then read before anything is published on it.
    let edited = lock
        .mismatches(&contract, &writer)
        .contains(&Mismatch::DataToml);

    let mut moved = false;
    let mut entries = Vec::new();
    for entry in contract.entries() {
        if !matches!(entry.access, Access::Write | Access::Replace) {
            continue;
        }
        let key = entry.key();
        let locked = lock.packages.get(&key);
        let reads = contract
            .entries()
            .iter()
            .find(|read| read.access == Access::Read && read.key() == key);
        let (read, published) = if locked.is_some_and(|locked| !locked.commits.is_empty()) {
            (None, EntryPublish::Unpushed)
        } else if let Some(reads) = reads {
            let base = locked.map(|locked| locked.base.as_str());
            Box::pin(publish_on_base(
                storage, remote, entry, reads, base, edited, user_rules,
            ))
            .await
        } else {
            let published = Box::pin(publish(
                storage,
                remote,
                entry,
                None,
                Base::Latest,
                user_rules,
            ))
            .await;
            (None, ran(published))
        };
        let published_base = match &published {
            EntryPublish::Ran(published) => published.lock_base(),
            _ => None,
        };
        if let Some(base) = published_base.or(read.as_ref().and_then(EntryRead::lock_base)) {
            lock.packages.insert(key, based_on(base));
            moved = true;
        }
        entries.push(PackagePublish {
            entry: entry.clone(),
            read,
            published,
        });
    }

    if moved {
        lock.write(storage, &contract, &writer).await?;
    }
    Ok(DataspacePublish { entries })
}

/// Publishes `entry` on `base`, first reading `reads`, the package's `read`
/// entry, where `latest` is not `base`, or a `base` is not to be trusted,
/// `read_first`.
async fn publish_on_base(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entry: &Entry,
    reads: &Entry,
    base: Option<&str>,
    read_first: bool,
    user_rules: &Path,
) -> (Option<EntryRead>, EntryPublish) {
    if !(read_first && base.is_some()) {
        match publish(storage, remote, entry, None, Base::Locked(base), user_rules).await {
            Ok(Published::Behind { .. }) => {}
            published => return (None, ran(published)),
        }
    }
    let read = match read(storage, remote, reads, base).await {
        Ok(outcome) => EntryRead::Ran(outcome),
        Err(err) => EntryRead::Failed(err),
    };
    let Some(base) = read.lock_base() else {
        return (Some(read), EntryPublish::NotRead);
    };
    let published = publish(
        storage,
        remote,
        entry,
        None,
        Base::Locked(Some(base)),
        user_rules,
    )
    .await;
    (Some(read), ran(published))
}

fn ran(published: Result<Published, EntryError>) -> EntryPublish {
    match published {
        Ok(published) => EntryPublish::Ran(published),
        Err(err) => EntryPublish::Failed(err),
    }
}

fn based_on(base: &str) -> LockEntry {
    LockEntry {
        base: base.to_string(),
        commits: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
