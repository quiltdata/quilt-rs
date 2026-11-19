//! `sync`: the tick. Each `sync` entry of a dataspace is read, then its local
//! work committed and published, its base and current revision taken from
//! the lock and its paths from the contract.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use chrono::SubsecRound;
use chrono::Utc;
use thiserror::Error;

use crate::LogicalKey;
use crate::Res;
use crate::dataspace::EntryError;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::publish::Published;
use crate::dataspace::publish::metadata_file;
use crate::dataspace::publish::same_as;
use crate::dataspace::publish::settle;
use crate::dataspace::publish::walk;
use crate::dataspace::read::OnDisk;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read::ReadReport;
use crate::dataspace::read::gone;
use crate::dataspace::read::read;
use crate::dataspace::read_all::usable_lock;
use crate::domain::DomainRoot;
use crate::domain::RegistryStore;
use crate::flow::PushTarget;
use crate::flow::WorkflowCheck;
use crate::flow::cache_manifest_at;
use crate::flow::push_files;
use crate::flow::record;
use crate::io::manifest::LatestSeen;
use crate::io::manifest::RowContent;
use crate::io::manifest::read_latest;
use crate::io::remote::Remote;
use crate::io::remote::WORKFLOWS_CONFIG_KEY;
use crate::io::remote::WorkflowIntent;
use crate::io::remote::resolve_workflow;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::HeaderMessage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::object_hash::ObjectHash;
use quilt_uri::ManifestUri;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageHandle;
use quilt_uri::S3Uri;

/// How one `sync` entry's tick ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum EntrySync {
    /// The tick ran: `read`, where there was a revision to read, then
    /// `published`, where the folder or the lock held local work. A read
    /// that could not place every file is reported and nothing published.
    Ran {
        read: Option<ReadReport>,
        published: Option<Published>,
    },
    /// Local changes collide with the revision's, so nothing was applied or
    /// published.
    Blocked {
        revision: String,
        conflicts: Vec<PathBuf>,
    },
    /// `latest` is neither the lock's base nor its last commit, and the lock
    /// names commits not pushed, or `latest` is gone: nothing was read or
    /// published.
    Diverged { latest: Option<String> },
    /// Another process holds the package in the Domain, so it was left for
    /// the next tick.
    Busy,
    /// The tick stopped; the lock keeps what completed before.
    Failed(EntryError),
}

/// What [`sync`] did, entry by entry.
#[derive(Debug)]
pub struct DataspaceSync {
    /// Each `sync` entry, in the contract's order, with how its tick ended.
    pub entries: Vec<(Entry, EntrySync)>,
    /// Whether the tick only read, as it does until the dataspace is filled,
    /// and after `data.toml` is edited until every entry is read again: a
    /// missing file counts as deleted only in a folder read in full against
    /// the contract as it stands.
    pub read_only: bool,
}

/// Why a dataspace was not synced at all, or stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SyncError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    /// The lock could not be read, or written after a step completed.
    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{} was written for another {}, and a working folder holds files or the lock names local commits not pushed; remove the lock, or fix it, to sync",
        crate::dataspace::lock::LOCK_FILE,
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),
}

impl From<ContractError> for SyncError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for SyncError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

/// Why one entry's tick stopped: the entry failed, or the lock could not be
/// written, which stops the whole tick.
enum Stop {
    Entry(EntryError),
    Lock(LockError),
}

impl From<EntryError> for Stop {
    fn from(err: EntryError) -> Self {
        Self::Entry(err)
    }
}

impl From<crate::Error> for Stop {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Writes the lock after each step, so an interrupted tick leaves it naming
/// what completed.
struct LockFile<'a, S> {
    storage: &'a S,
    contract: &'a Contract,
    writer: &'a Writer,
}

impl<S: Storage + Sync> LockFile<'_, S> {
    async fn save(&self, lock: &mut Lock) -> Result<(), Stop> {
        lock.write(self.storage, self.contract, self.writer)
            .await
            .map_err(Stop::Lock)
    }
}

/// Ticks each `sync` entry of the dataspace in the folder `dataspace`: reads
/// the revision `latest` names into its working folder where it moved past
/// the lock's base, then commits the folder's changes under the entry's
/// paths, deletions included, as a local commit in `domain`'s store for the
/// bucket, and pushes the lock's last commit against its base. The lock is
/// written after each step: the base after a read, a commit once it is
/// recorded, and the base again, with the commits the push squashed dropped,
/// once `latest` names the push. A push that `latest` moved past is left
/// stored, and the lock where it was.
///
/// Until the dataspace is filled, and after `data.toml` is edited, the tick
/// only reads: each entry is read in full, a missing file filled rather than
/// taken for a deletion, and nothing new is committed; commits the lock
/// already names are pushed first. `filled` is set once
/// every `read` and `sync` entry is in the lock, and the lock takes the
/// edited contract only once every `sync` entry was read whole.
///
/// The lock is used as [`read_all`](crate::dataspace::read_all::read_all)
/// uses it. Each commit's message is `message`, and its workflow the
/// bucket's default.
pub async fn sync(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    message: &str,
) -> Result<DataspaceSync, SyncError> {
    let contract = Contract::read(storage, dataspace).await?;
    let (mut lock, writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(SyncError::Mismatch)?;
    let lock_file = LockFile {
        storage,
        contract: &contract,
        writer: &writer,
    };
    let synced: Vec<&Entry> = contract
        .entries()
        .iter()
        .filter(|entry| entry.access == Access::Sync)
        .collect();
    let edited = lock.header.data_toml_hash != contract.data_toml_hash();

    let mut entries = Vec::new();
    if edited || lock.header.filled.is_none() {
        for entry in synced {
            let outcome = match fill(storage, remote, domain, entry, &mut lock).await {
                Ok(outcome) => outcome,
                Err(err) => EntrySync::Failed(err),
            };
            entries.push((entry.clone(), outcome));
        }
        let whole = entries.iter().all(|(_, outcome)| match outcome {
            EntrySync::Ran {
                read,
                published: None | Some(Published::Advanced { .. }),
            } => read
                .as_ref()
                .is_none_or(|read| read.skipped.is_empty() && read.hash_mismatch.is_empty()),
            _ => false,
        });
        if lock.header.filled.is_none() && filled(&contract, &lock, &entries) {
            lock.header.filled = Some(Utc::now().trunc_subsecs(0));
        }
        // A lock written for the edited contract would let absence count
        // under paths not yet read.
        if !edited || whole {
            lock.write(storage, &contract, &writer).await?;
        }
        return Ok(DataspaceSync {
            entries,
            read_only: true,
        });
    }

    for entry in synced {
        let outcome = match tick(
            storage, remote, domain, entry, &mut lock, &lock_file, message,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(Stop::Entry(err)) => EntrySync::Failed(err),
            Err(Stop::Lock(err)) => return Err(err.into()),
        };
        entries.push((entry.clone(), outcome));
    }
    Ok(DataspaceSync {
        entries,
        read_only: false,
    })
}

/// Whether every `read` and `sync` entry is in the lock, or, under `sync`,
/// has no revision to read.
fn filled(contract: &Contract, lock: &Lock, synced: &[(Entry, EntrySync)]) -> bool {
    let no_revision = |entry: &Entry| {
        synced.iter().any(|(synced, outcome)| {
            synced == entry && matches!(outcome, EntrySync::Ran { read: None, .. })
        })
    };
    contract
        .entries()
        .iter()
        .filter(|entry| matches!(entry.access, Access::Read | Access::Sync))
        .all(|entry| lock.packages.contains_key(&entry.key()) || no_revision(entry))
}

/// Reads `entry` in full at `latest`, filling what is missing, and moves its
/// base in `lock` where the read completes. Commits the lock names were made
/// against the contract they were committed under, so they are pushed first,
/// with no new commit; one that does not become `latest` leaves the entry
/// unread.
async fn fill(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    entry: &Entry,
    lock: &mut Lock,
) -> Result<EntrySync, EntryError> {
    let key = entry.key();
    let mut published = None;
    if let Some(locked) = lock
        .packages
        .get(&key)
        .filter(|locked| !locked.commits.is_empty())
    {
        let Registry::S3(bucket) = &entry.registry;
        let store = domain.registry(bucket)?;
        let Some(_held) = storage
            .try_lock_exclusive(store.package_lock(&entry.package))
            .await?
        else {
            return Ok(EntrySync::Busy);
        };
        let head = locked.commits.last().map_or("", String::as_str);
        let pushed = push_head(storage, remote, &store, entry, head, Some(&locked.base)).await?;
        let Published::Advanced { revision, .. } = &pushed else {
            return Ok(EntrySync::Ran {
                read: None,
                published: Some(pushed),
            });
        };
        let locked = LockEntry {
            base: revision.clone(),
            commits: Vec::new(),
        };
        lock.packages.insert(key.clone(), locked);
        published = Some(pushed);
    }
    let base = lock.packages.get(&key).map(|locked| locked.base.clone());
    let Some(latest) = latest(remote, entry).await? else {
        return Ok(match base {
            None => EntrySync::Ran {
                read: None,
                published: None,
            },
            Some(_) => EntrySync::Diverged { latest: None },
        });
    };
    let mut whole = at(entry, &latest);
    whole.access = Access::Read;
    let outcome = read(storage, remote, &whole, base.as_deref()).await?;
    if let Some(revision) = outcome.lock_base() {
        let locked = LockEntry {
            base: revision.to_string(),
            commits: Vec::new(),
        };
        lock.packages.insert(key, locked);
    }
    Ok(match outcome {
        ReadOutcome::Read(report) => EntrySync::Ran {
            read: Some(report),
            published,
        },
        ReadOutcome::Blocked {
            revision,
            conflicts,
        } => EntrySync::Blocked {
            revision,
            conflicts,
        },
    })
}

/// The hash `latest` names for `entry`'s package, `None` where it has none.
async fn latest(remote: &impl Remote, entry: &Entry) -> Result<Option<String>, EntryError> {
    match read_latest(remote, entry.catalog.as_ref(), handle(entry)).await? {
        LatestSeen::Tagged { uri, .. } => Ok(Some(uri.hash)),
        LatestSeen::Absent => Ok(None),
        LatestSeen::Unreadable => Err(EntryError::LatestUnreadable),
    }
}

/// `entry` pinned to the revision `hash`, so a read takes the `latest` the
/// tick decided on rather than reading the tag again.
fn at(entry: &Entry, hash: &str) -> Entry {
    let mut pinned = entry.clone();
    pinned.revision = RevisionPointer::Hash(hash.to_string());
    pinned
}

fn handle(entry: &Entry) -> S3PackageHandle {
    let Registry::S3(bucket) = &entry.registry;
    S3PackageHandle {
        bucket: bucket.clone(),
        namespace: entry.package.clone(),
    }
}

/// One entry's tick; see [`sync`].
async fn tick(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    message: &str,
) -> Result<EntrySync, Stop> {
    let Registry::S3(bucket) = &entry.registry;
    let store = domain.registry(bucket)?;
    let Some(_held) = storage
        .try_lock_exclusive(store.package_lock(&entry.package))
        .await?
    else {
        return Ok(EntrySync::Busy);
    };
    let key = entry.key();
    let (base, commits) = match lock.packages.get(&key) {
        Some(locked) => (Some(locked.base.clone()), locked.commits.clone()),
        None => (None, Vec::new()),
    };
    let latest = latest(remote, entry).await?;

    let mut read_report = None;
    if latest == base {
        // Nothing to read.
    } else if let Some(pushed) = latest
        .as_ref()
        .filter(|latest| commits.last() == Some(latest))
    {
        // The push landed and the lock was not written after it.
        set_base(lock, lock_file, &key, pushed.clone()).await?;
    } else if base.is_some() && (latest.is_none() || !commits.is_empty()) {
        return Ok(EntrySync::Diverged { latest });
    } else if let Some(hash) = &latest {
        let outcome = read(storage, remote, &at(entry, hash), base.as_deref()).await?;
        let completed = outcome.lock_base().map(str::to_string);
        let report = match outcome {
            ReadOutcome::Read(report) => report,
            ReadOutcome::Blocked {
                revision,
                conflicts,
            } => {
                return Ok(EntrySync::Blocked {
                    revision,
                    conflicts,
                });
            }
        };
        let Some(revision) = completed else {
            return Ok(EntrySync::Ran {
                read: Some(report),
                published: None,
            });
        };
        set_base(lock, lock_file, &key, revision).await?;
        read_report = Some(report);
    }

    let published = publish(storage, remote, &store, entry, lock, lock_file, message).await?;
    Ok(EntrySync::Ran {
        read: read_report,
        published,
    })
}

/// Moves `key`'s base to `base`, dropping its commits, and writes the lock.
async fn set_base(
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    key: &crate::dataspace::contract::PackageKey,
    base: String,
) -> Result<(), Stop> {
    lock.packages.insert(
        key.clone(),
        LockEntry {
            base,
            commits: Vec::new(),
        },
    );
    lock_file.save(lock).await
}

/// Commits the working folder's changes on the lock's current revision, then
/// pushes the last commit against the base. `None` where there was nothing
/// to commit or push.
async fn publish(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &RegistryStore,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    message: &str,
) -> Result<Option<Published>, Stop> {
    let key = entry.key();
    let (base, commits) = match lock.packages.get(&key) {
        Some(locked) => (Some(locked.base.clone()), locked.commits.clone()),
        None => (None, Vec::new()),
    };
    let current_hash = commits.last().or(base.as_ref());
    let current = match current_hash {
        None => Manifest::default(),
        Some(hash) if commits.last() == Some(hash) => {
            Manifest::from_path(storage, &store.installed_manifest(&entry.package, hash)).await?
        }
        Some(hash) => base_manifest(storage, remote, store, entry, hash).await?,
    };

    let folder = &entry.folder;
    if current_hash.is_some() && gone(storage, folder, &current, &entry.paths).await? {
        return Err(EntryError::FolderGone(folder.clone()).into());
    }
    let files: Vec<(LogicalKey, PathBuf)> = walk(storage, folder)
        .await?
        .into_iter()
        .filter(|(key, _)| entry.paths.covers(key))
        .collect();
    let (mut manifest, content) = compose(storage, &current, entry, &files).await?;
    metadata_file(storage, folder)
        .await?
        .apply(&mut manifest, &entry.paths);

    let mut head = current_hash.cloned();
    // The tick's message wins over the metadata file's, so the file's alone
    // is no change.
    if !same_as(&manifest, current_hash.map(|_| &current), false) {
        let Registry::S3(bucket) = &entry.registry;
        let workflows = S3Uri {
            bucket: bucket.clone(),
            key: WORKFLOWS_CONFIG_KEY.to_string(),
            version: None,
        };
        manifest.header.message = HeaderMessage::Text(message.to_string());
        manifest.header.workflow = resolve_workflow(
            remote,
            entry.catalog.as_ref(),
            WorkflowIntent::BucketDefault,
            &workflows,
        )
        .await?;
        let lineage = PackageLineage {
            base_hash: current_hash.cloned().unwrap_or_default(),
            ..PackageLineage::default()
        };
        let (_, recorded) = record(
            store,
            storage,
            remote,
            WorkflowCheck::Warn {
                host: entry.catalog.as_ref(),
            },
            &entry.package,
            lineage,
            manifest,
            &content,
        )
        .await?;
        if let Some(hash) = recorded.hash() {
            crate::failpoint::stop("sync.recorded")?;
            // A package not yet published has no base for the lock to hold
            // its commit against, so the commit is pushed at once.
            if let Some(locked) = lock.packages.get_mut(&key) {
                locked.commits.push(hash.to_string());
                lock_file.save(lock).await?;
            }
            head = Some(hash.to_string());
        }
    }

    let Some(head) = head.filter(|head| Some(head) != base.as_ref()) else {
        return Ok(None);
    };
    let published = push_head(storage, remote, store, entry, &head, base.as_deref()).await?;
    if let Published::Advanced { revision, .. } = &published {
        crate::failpoint::stop("sync.pushed")?;
        set_base(lock, lock_file, &key, revision.clone()).await?;
    }
    Ok(Some(published))
}

/// Pushes the local commit `head` against `base`, squashing the commits
/// before it.
async fn push_head(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &RegistryStore,
    entry: &Entry,
    head: &str,
    base: Option<&str>,
) -> Result<Published, EntryError> {
    let manifest =
        Manifest::from_path(storage, &store.installed_manifest(&entry.package, head)).await?;
    let target = PushTarget {
        handle: handle(entry),
        origin: entry.catalog.clone(),
        parent: base.map(str::to_string),
    };
    let pushed = Box::pin(push_files(
        storage,
        remote,
        &target,
        manifest,
        &BTreeMap::new(),
    ))
    .await;
    settle(remote, &target, pushed).await
}

/// The manifest of `hash`, a revision of `entry`'s package in its bucket,
/// kept in `store` after its first fetch.
async fn base_manifest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &RegistryStore,
    entry: &Entry,
    hash: &str,
) -> Res<Manifest> {
    let Registry::S3(bucket) = &entry.registry;
    let uri = ManifestUri {
        origin: entry.catalog.clone(),
        bucket: bucket.clone(),
        namespace: entry.package.clone(),
        hash: hash.to_string(),
    };
    cache_manifest_at(storage, remote, &uri, &store.cached_manifest(hash)).await
}

/// The manifest the folder holds: `current` with its rows under the entry's
/// paths replaced by `files`, a row whose file is gone dropped, and the
/// content of each row a file changed or added. A file that still holds its
/// row's bytes keeps the row; a changed one is left for [`record`] to hash.
async fn compose(
    storage: &(impl Storage + Sync),
    current: &Manifest,
    entry: &Entry,
    files: &[(LogicalKey, PathBuf)],
) -> Res<(Manifest, BTreeMap<LogicalKey, RowContent>)> {
    let mut on_disk: BTreeMap<&LogicalKey, &PathBuf> =
        files.iter().map(|(key, path)| (key, path)).collect();
    let mut rows = Vec::new();
    let mut content = BTreeMap::new();
    for row in &current.rows {
        if !entry.paths.covers(&row.logical_key) {
            rows.push(row.clone());
            continue;
        }
        let Some(path) = on_disk.remove(&row.logical_key) else {
            continue;
        };
        if OnDisk::read(storage, path, row.hash.algorithm())
            .await?
            .holds(row)
        {
            rows.push(row.clone());
        } else {
            rows.push(changed(row.logical_key.clone(), Some(row)));
            content.insert(row.logical_key.clone(), RowContent::Path(path.clone()));
        }
    }
    for (key, path) in on_disk {
        rows.push(changed(key.clone(), None));
        content.insert(key.clone(), RowContent::Path(path.clone()));
    }
    rows.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    let manifest = Manifest {
        header: current.header.clone(),
        rows,
        dir_meta: current.dir_meta.clone(),
    };
    Ok((manifest, content))
}

/// A row not yet hashed, keeping `was`'s metadata.
fn changed(logical_key: LogicalKey, was: Option<&ManifestRow>) -> ManifestRow {
    ManifestRow {
        logical_key,
        physical_key: String::new(),
        hash: ObjectHash::default(),
        size: 0,
        meta: was.and_then(|row| row.meta.clone()),
        meta_numbers: was.map_or_else(RawNumbers::default, |row| row.meta_numbers.clone()),
    }
}

#[cfg(test)]
mod tests;
