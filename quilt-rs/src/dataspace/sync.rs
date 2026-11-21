//! `sync`: the tick. Each `sync` entry of a dataspace is read, then its local
//! work committed and published, its base and current revision taken from
//! the lock and its paths from the contract.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use tracing::warn;

use crate::LogicalKey;
use crate::Res;
use crate::WorkflowValidationError;
use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::conflict::read_resolving;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::hooks;
use crate::dataspace::hooks::HookPoint;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::metadata::render;
use crate::dataspace::publish::Published;
use crate::dataspace::publish::metadata_file;
use crate::dataspace::publish::same_as;
use crate::dataspace::publish::settle;
use crate::dataspace::publish::walk;
use crate::dataspace::read::OnDisk;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read::ReadReport;
use crate::dataspace::read::STAGING_PREFIX;
use crate::dataspace::read::bytes_at;
use crate::dataspace::read::gone;
use crate::dataspace::read::write_metadata;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::Store;
use crate::dataspace::store::local_registry;
use crate::dataspace::store::name_local;
use crate::domain::DomainRoot;
use crate::domain::LocalPublish;
use crate::domain::LocalRegistry;
use crate::flow::PackageStore;
use crate::flow::PushTarget;
use crate::flow::WorkflowCheck;
use crate::flow::WorkflowWarning;
use crate::flow::cache_manifest_at;
use crate::flow::check_workflow;
use crate::flow::install_to;
use crate::flow::record;
use crate::flow::refuse_linked_ancestors;
use crate::flow::store_files;
use crate::flow::tag_stored;
use crate::io::manifest::LatestSeen;
use crate::io::manifest::RowContent;
use crate::io::manifest::read_latest;
use crate::io::remote::Remote;
use crate::io::remote::WORKFLOWS_CONFIG_KEY;
use crate::io::remote::WorkflowIntent;
use crate::io::remote::fetch_workflows_config;
use crate::io::remote::resolve_workflow;
use crate::io::remote::resolve_workflow_from_config;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::HeaderMessage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::manifest::Workflow;
use crate::object_hash::ObjectHash;
use crate::queue::KeptReason;
use crate::queue::KeptRevision;
use crate::queue::Queue;
use crate::quiltignore::Rules;
use crate::state_store::FolderKey;
use quilt_uri::ManifestUri;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageHandle;
use quilt_uri::S3Uri;
use quilt_uri::Tag;

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
    /// Local changes collide with the revision's, that being read or the
    /// `latest` local commits were to be merged onto, so nothing was applied
    /// or published; `conflict.json` holds the colliding keys.
    Blocked {
        revision: String,
        conflicts: Vec<PathBuf>,
    },
    /// `latest` is gone while the lock names a base, or the package is moving
    /// from the local registry to a bucket that has a `latest` for it
    /// already: nothing was read or published.
    Diverged { latest: Option<String> },
    /// The working folder was missing or empty, so the lock's current
    /// revision was read back into it and nothing was published.
    ReadBack(ReadReport),
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
/// what completed, written for the `data.toml` whose hash is `written_for`.
struct LockFile<'a, S> {
    storage: &'a S,
    contract: &'a Contract,
    writer: &'a Writer,
    written_for: String,
}

impl<S: Storage + Sync> LockFile<'_, S> {
    async fn save(&self, lock: &mut Lock) -> Result<(), Stop> {
        lock.write_for(self.storage, self.contract, self.writer, &self.written_for)
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
/// once `latest` names the push. A package never published is recorded with
/// its commits and no base, and its first push expects no `latest`. Where
/// `latest` moved past the base of commits the lock names, or appeared
/// before a package's first push, the commits and the folder's changes are
/// merged onto it, the empty revision standing for a missing base: where
/// nothing collides the folder takes `latest`'s change, and the merge is one
/// commit on `latest`, pushed as any is; where something does, nothing is
/// applied and `conflict.json` holds a publish's row for each colliding key,
/// whose decisions a later tick applies. Commits are not checked
/// against the bucket's workflow; their push is. A push that `latest` moved
/// past, or that the workflow refuses, is left stored, recorded with
/// [`Queue::keep`], and not uploaded again while the bucket holds it; the
/// lock stays where it was. A working folder the lock says was
/// filled and that is now missing or empty is read back at the lock's
/// current revision, its last commit or else its base, and nothing is
/// published. What the ignore rules leave out of a commit is not committed,
/// `user_rules` being the user-wide file.
///
/// Until the dataspace is filled, and after `data.toml` is edited, the tick
/// only reads: each entry is read in full, a missing file filled rather than
/// taken for a deletion, and nothing new is committed; commits the lock
/// already names are pushed first, or, where `latest` moved past their base,
/// merged onto it without the folder's changes and read whole, the merge
/// left for the first tick after to push. `filled` is set once
/// every `read` and `sync` entry is in the lock, and the lock takes the
/// edited contract only once every `sync` entry was read whole.
///
/// An entry of `domain`'s local registry, under its name or one it had, is
/// ticked as a bucket's is, with the registry as both its store and its
/// registry: its commits are published there against the same base, under
/// the package's lock and with no network, and its key is written under the
/// registry's current name. An entry naming another local registry is
/// refused, and keeps the dataspace unfilled.
///
/// A package `data.toml` was edited to move from this machine's local
/// registry to a bucket, which the lock still holds under the local
/// registry's key, is moved first: its last local commit is pushed as the
/// bucket's first revision, squashing the commits before it, and the lock
/// then holds the package under the bucket's key, based at that revision.
/// The local registry keeps the package. A bucket that has the package
/// already leaves it [`EntrySync::Diverged`], and a push that does not
/// advance `latest` leaves the lock as it was, to move on a later tick.
///
/// The lock is used as [`read_all`](crate::dataspace::read_all::read_all)
/// uses it, and `conflict.json` as [`read_resolving`] does. Each commit's
/// message is `message`, and its workflow the bucket's default, or none in
/// the local registry.
///
/// The contract's hooks run around each half: `pre-read` before the tick
/// reads `latest` or merges onto it, and `post-read` once the lock moved to a
/// newer revision;
/// `pre-write` before it commits a folder that changed, which is then taken
/// again, and `post-write` once a push stored a revision or moved `latest`.
/// A `pre-` hook that fails stops the entry. Reading a missing folder back
/// runs none.
#[allow(clippy::too_many_arguments)]
pub async fn sync(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    queue: &Queue,
    dataspace: &Path,
    writer: &Writer,
    message: &str,
    user_rules: &Path,
) -> Result<DataspaceSync, SyncError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(SyncError::Mismatch)?;
    let local = domain.local();
    name_local(storage, Some(&local), &mut contract, &mut lock, &mut writer).await?;
    let synced: Vec<&Entry> = contract
        .entries()
        .iter()
        .filter(|entry| entry.access == Access::Sync)
        .collect();
    let edited = lock.header.data_toml_hash != contract.data_toml_hash();
    // A lock written for the edited contract would let absence count under
    // paths not yet read, so it keeps naming the one before until they are.
    let lock_file = LockFile {
        storage,
        contract: &contract,
        writer: &writer,
        written_for: lock.header.data_toml_hash.clone(),
    };

    let mut entries = Vec::new();
    if edited || lock.header.filled.is_none() {
        for entry in synced {
            let outcome =
                match fill(storage, remote, domain, queue, entry, &mut lock, &lock_file).await {
                    Ok(outcome) => outcome,
                    Err(Stop::Entry(err)) => EntrySync::Failed(err),
                    Err(Stop::Lock(err)) => return Err(err.into()),
                };
            entries.push((entry.clone(), outcome));
        }
        let whole = entries.iter().all(|(_, outcome)| match outcome {
            // A move the workflow refused read nothing: the folder is still
            // to be read whole once it goes through.
            EntrySync::Ran {
                read: None,
                published: Some(Published::WorkflowBlocked { .. }),
            } => false,
            EntrySync::Ran {
                read,
                published:
                    None | Some(Published::Advanced { .. } | Published::WorkflowBlocked { .. }),
            } => read.as_ref().is_none_or(ReadReport::complete),
            _ => false,
        });
        // A `read` entry that cannot be asked about keeps the dataspace
        // unfilled; the next tick asks again.
        if lock.header.filled.is_none()
            && filled(storage, remote, &local, &contract, &lock, whole)
                .await
                .unwrap_or(false)
        {
            lock.header.filled = Some(crate::time::to_second(jiff::Timestamp::now()));
        }
        if whole {
            lock.write_for(storage, &contract, &writer, contract.data_toml_hash())
                .await?;
        } else if let Err(Stop::Lock(err)) = lock_file.save(&mut lock).await {
            return Err(err.into());
        }
        return Ok(DataspaceSync {
            entries,
            read_only: true,
        });
    }

    for entry in synced {
        let outcome = match tick(
            storage, remote, domain, queue, entry, &mut lock, &lock_file, message, user_rules,
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

/// Whether every `sync` entry was read whole by this fill, a lock carried
/// from elsewhere naming packages whose folders this one may not hold, and
/// every `read` entry is in the lock or has no revision to read.
async fn filled(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: &LocalRegistry,
    contract: &Contract,
    lock: &Lock,
    whole: bool,
) -> Result<bool, EntryError> {
    if !whole {
        return Ok(false);
    }
    for entry in contract.entries() {
        // A pinned entry has a revision to read whatever `latest` names.
        if entry.access == Access::Read
            && !lock.packages.contains_key(&entry.key())
            && (entry.revision != RevisionPointer::Tag(Tag::Latest)
                || latest_in(storage, remote, Some(local), entry)
                    .await?
                    .is_some())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Reads `entry` in full at `latest`, filling what is missing, and moves its
/// base where the read completes. Commits the lock names were made against
/// the contract they were committed under, so they are pushed first, with no
/// new commit, where `latest` is still their base; one the workflow refuses
/// is read at instead. Commits `latest` moved past are merged onto it alone,
/// the folder's changes left out, and the merge read whole, as
/// [`merge::filling`] does, to be pushed once the dataspace is filled.
async fn fill(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    queue: &Queue,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
) -> Result<EntrySync, Stop> {
    let store = Store::of(storage, domain, entry).await?;
    let Some(_held) = storage
        .try_lock_exclusive(store.package_lock(&entry.package))
        .await?
    else {
        return Ok(EntrySync::Busy);
    };
    let key = entry.key();
    let mut published =
        match move_to_bucket(storage, remote, domain, queue, entry, lock, lock_file).await? {
            ControlFlow::Break(outcome) => return Ok(outcome),
            ControlFlow::Continue(pushed) => pushed,
        };
    let local = store.local();
    let mut latest = latest_in(storage, remote, local, entry).await?;
    if let Some(locked) = lock.packages.get(&key).cloned()
        && let Some(head) = locked.commits.last()
    {
        if latest.as_ref() == Some(head) {
            // The push landed and the lock was not written after it.
            set_base(lock, lock_file, &key, head.clone()).await?;
        } else if latest != locked.base {
            let Some(theirs) = latest.as_deref() else {
                return Ok(EntrySync::Diverged { latest });
            };
            // Commits on a base `latest` moved from, or on none where
            // `latest` now names a revision: merged onto it, then read whole.
            return merge::filling(storage, remote, &store, entry, lock, lock_file, theirs).await;
        } else {
            let (pushed, wrote) = push_head(
                storage,
                remote,
                &store,
                queue,
                entry,
                head,
                locked.base.as_deref(),
            )
            .await?;
            if let Published::Advanced { revision, .. } = &pushed {
                set_base(lock, lock_file, &key, revision.clone()).await?;
            }
            if wrote {
                after_write(lock_file.contract, entry, &pushed).await;
            }
            match &pushed {
                Published::Advanced { revision, .. } => {
                    latest = Some(revision.clone());
                    published = Some(pushed);
                }
                // The bucket holds the refused commit, so the folder is read
                // whole at it, and the lock keeps it for a fix to build on.
                Published::WorkflowBlocked { revision, .. } => {
                    let mut whole = at(entry, revision);
                    whole.access = Access::Read;
                    let dataspace = lock_file.contract.dataspace();
                    let outcome =
                        read_resolving(storage, remote, local, dataspace, &whole, Some(revision))
                            .await?;
                    return Ok(ran(outcome, Some(pushed)));
                }
                _ => {
                    return Ok(EntrySync::Ran {
                        read: None,
                        published: Some(pushed),
                    });
                }
            }
        }
    }
    let base = lock
        .packages
        .get(&key)
        .and_then(|locked| locked.base.clone());
    let Some(latest) = latest else {
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
    let contract = lock_file.contract;
    hooks::run(contract, HookPoint::PreRead, entry, None)
        .await
        .map_err(EntryError::from)?;
    let dataspace = contract.dataspace();
    let outcome =
        read_resolving(storage, remote, local, dataspace, &whole, base.as_deref()).await?;
    if let Some(revision) = outcome.lock_base() {
        set_base(lock, lock_file, &key, revision.to_string()).await?;
    }
    if let Some(revision) = hooks::newer(base.as_deref(), outcome.lock_base()) {
        hooks::after(contract, HookPoint::PostRead, entry, revision).await;
    }
    Ok(ran(outcome, published))
}

/// Runs the contract's `post-write` hook for the revision `published`
/// stored, if any.
async fn after_write(contract: &Contract, entry: &Entry, published: &Published) {
    if let Some(revision) = published.stored() {
        hooks::after(contract, HookPoint::PostWrite, entry, revision).await;
    }
}

fn ran(outcome: ReadOutcome, published: Option<Published>) -> EntrySync {
    match outcome {
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
    }
}

/// The hash `latest` names for `entry`'s package, `None` where it has none.
pub(crate) async fn latest(
    remote: &impl Remote,
    entry: &Entry,
) -> Result<Option<String>, EntryError> {
    match read_latest(remote, entry.catalog.as_ref(), handle(entry)?).await? {
        LatestSeen::Tagged { uri, .. } => Ok(Some(uri.hash)),
        LatestSeen::Absent => Ok(None),
        LatestSeen::Unreadable => Err(EntryError::LatestUnreadable),
    }
}

/// [`latest`], read from the local registry `local` for an entry of it.
pub(crate) async fn latest_in(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    entry: &Entry,
) -> Result<Option<String>, EntryError> {
    match local_registry(storage, local, entry).await? {
        Some(registry) => Ok(registry.read_latest(storage, &entry.package).await?),
        None => latest(remote, entry).await,
    }
}

/// `entry` pinned to the revision `hash`, so a read takes the `latest` the
/// tick decided on rather than reading the tag again.
fn at(entry: &Entry, hash: &str) -> Entry {
    let mut pinned = entry.clone();
    pinned.revision = RevisionPointer::Hash(hash.to_string());
    pinned
}

fn handle(entry: &Entry) -> Result<S3PackageHandle, EntryError> {
    Ok(S3PackageHandle {
        bucket: bucket(entry)?.to_string(),
        namespace: entry.package.clone(),
    })
}

/// `entry`'s bucket; a local registry's package, which has none, is refused.
fn bucket(entry: &Entry) -> Result<&str, EntryError> {
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry);
    };
    Ok(bucket)
}

/// Where `data.toml` was edited to move `entry`'s package from this
/// machine's local registry to its bucket: the lock holds the package under
/// the local registry's key and not under the bucket's, and no entry of
/// `contract` names the local one. The local key, with the package's last
/// local commit. `writer` names the local registry.
pub(crate) fn moved_from(
    lock: &Lock,
    contract: &Contract,
    writer: &Writer,
    entry: &Entry,
) -> Option<(PackageKey, String)> {
    let name = writer.local_registry_name()?;
    if !matches!(entry.registry, Registry::S3(_)) || lock.packages.contains_key(&entry.key()) {
        return None;
    }
    let local = PackageKey {
        registry: Registry::Local(name.to_string()),
        package: entry.package.clone(),
    };
    if contract.entries().iter().any(|other| other.key() == local) {
        return None;
    }
    let locked = lock.packages.get(&local)?;
    Some((local, locked.current()?.to_string()))
}

/// `entry` as the lock still holds it: where [`moved_from`] finds its
/// package's move to its bucket not landed yet, the entry under the local
/// registry's key, whose store holds its commits until the move pushes the
/// last of them; else `entry`. What `commit` and `undo_commit` act on, so a
/// moved package is followed rather than started again in the bucket.
pub(crate) fn pending_move(
    lock: &Lock,
    contract: &Contract,
    writer: &Writer,
    entry: Entry,
) -> Entry {
    match moved_from(lock, contract, writer, &entry) {
        Some((local, _)) => Entry {
            registry: local.registry,
            ..entry
        },
        None => entry,
    }
}

/// Moves `entry`'s package to its bucket where `data.toml` was edited to,
/// as [`moved_from`] tells: its last local commit is the bucket's first
/// revision, the commits before it squashed, and the lock then holds the
/// package under the bucket's key, based at it. Continues with the push,
/// `None` where there was no move or its push had landed already; breaks
/// with how the tick ends where the bucket's `latest` names another
/// revision, or the push did not advance it, the lock left as it was.
#[allow(clippy::too_many_arguments)]
async fn move_to_bucket(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    queue: &Queue,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
) -> Result<ControlFlow<EntrySync, Option<Published>>, Stop> {
    let Some((local, head)) = moved_from(lock, lock_file.contract, lock_file.writer, entry) else {
        return Ok(ControlFlow::Continue(None));
    };
    let latest = latest(remote, entry).await?;
    let (base, pushed) = match latest.as_deref() {
        // The push landed and the lock was not written after it.
        Some(latest) if latest == head => (head, None),
        Some(_) => return Ok(ControlFlow::Break(EntrySync::Diverged { latest })),
        None => {
            let manifest = local_commit(storage, &domain.local(), entry, &head).await?;
            let (pushed, wrote) =
                push_manifest(storage, remote, queue, entry, &head, manifest, None).await?;
            if wrote {
                after_write(lock_file.contract, entry, &pushed).await;
            }
            let Published::Advanced { revision, .. } = &pushed else {
                return Ok(ControlFlow::Break(EntrySync::Ran {
                    read: None,
                    published: Some(pushed),
                }));
            };
            (revision.clone(), Some(pushed))
        }
    };
    lock.packages.remove(&local);
    set_base(lock, lock_file, &entry.key(), base).await?;
    Ok(ControlFlow::Continue(pushed))
}

/// The manifest of `hash`, a commit of `entry`'s package in `local`, or a
/// revision published there.
pub(crate) async fn local_commit(
    storage: &(impl Storage + Sync),
    local: &LocalRegistry,
    entry: &Entry,
    hash: &str,
) -> Result<Manifest, EntryError> {
    let installed = local.installed_manifest(&entry.package, hash);
    let path = if storage.exists(&installed).await {
        installed
    } else {
        local.revision(hash)
    };
    Ok(Manifest::from_path(storage, &path).await?)
}

/// One entry's tick; see [`sync`].
#[allow(clippy::too_many_arguments)]
async fn tick(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    queue: &Queue,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    message: &str,
    user_rules: &Path,
) -> Result<EntrySync, Stop> {
    let store = Store::of(storage, domain, entry).await?;
    let Some(_held) = storage
        .try_lock_exclusive(store.package_lock(&entry.package))
        .await?
    else {
        return Ok(EntrySync::Busy);
    };
    // A move a fill did not complete; its publish is the tick's.
    match move_to_bucket(storage, remote, domain, queue, entry, lock, lock_file).await? {
        ControlFlow::Break(outcome) => return Ok(outcome),
        ControlFlow::Continue(Some(pushed)) => {
            return Ok(EntrySync::Ran {
                read: None,
                published: Some(pushed),
            });
        }
        ControlFlow::Continue(None) => {}
    }
    let key = entry.key();
    let local = store.local();
    let (base, commits) = match lock.packages.get(&key) {
        Some(locked) => (locked.base.clone(), locked.commits.clone()),
        None => (None, Vec::new()),
    };
    let mut current = None;
    if let Some(hash) = commits.last().or(base.as_ref()) {
        let manifest = manifest_at(storage, remote, &store, entry, hash, &commits).await?;
        if gone(storage, &entry.folder, &manifest, &entry.paths).await? {
            let report = read_back(storage, remote, &store, entry, hash, &manifest).await?;
            return Ok(EntrySync::ReadBack(report));
        }
        current = Some((hash.clone(), manifest));
    }
    let latest = latest_in(storage, remote, local, entry).await?;

    let mut read_report = None;
    if latest == base {
        // Nothing to read.
    } else if let Some(pushed) = latest
        .as_ref()
        .filter(|latest| commits.last() == Some(latest))
    {
        // The push landed and the lock was not written after it.
        set_base(lock, lock_file, &key, pushed.clone()).await?;
    } else if !commits.is_empty()
        && let (Some(theirs), Some((_, manifest))) = (&latest, &current)
    {
        // Commits on a base `latest` moved from, or on none where `latest`
        // now names a revision: merged onto it.
        let ours = merge::Local {
            base: base.as_deref(),
            current: manifest,
        };
        match merge::onto_latest(
            storage, remote, &store, entry, lock, lock_file, ours, theirs, message, user_rules,
        )
        .await?
        {
            ControlFlow::Continue(report) => read_report = Some(report),
            ControlFlow::Break(outcome) => return Ok(outcome),
        }
    } else if base.is_some() && latest.is_none() {
        // A base whose `latest` is gone.
        return Ok(EntrySync::Diverged { latest });
    } else if let Some(hash) = &latest {
        let contract = lock_file.contract;
        hooks::run(contract, HookPoint::PreRead, entry, None)
            .await
            .map_err(EntryError::from)?;
        let dataspace = contract.dataspace();
        let pinned = at(entry, hash);
        let outcome =
            read_resolving(storage, remote, local, dataspace, &pinned, base.as_deref()).await?;
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
        set_base(lock, lock_file, &key, revision.clone()).await?;
        // `latest` moved past the base, so the revision read is newer.
        hooks::after(contract, HookPoint::PostRead, entry, &revision).await;
        read_report = Some(report);
    }

    // A base that moved names a revision the bucket holds, read from there.
    let known = current.filter(|_| latest == base);
    let published = publish(
        storage, remote, &store, queue, entry, lock, lock_file, known, message, user_rules,
    )
    .await?;
    Ok(EntrySync::Ran {
        read: read_report,
        published,
    })
}

/// Moves `key`'s base to `base`, dropping its commits, and writes the lock.
async fn set_base(
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    key: &PackageKey,
    base: String,
) -> Result<(), Stop> {
    lock.packages.insert(key.clone(), LockEntry::at(base));
    lock_file.save(lock).await
}

/// The workflow a commit records: the bucket's default, and none for the
/// local registry, which has no workflows.
async fn commit_workflow(
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
) -> Result<Option<Workflow>, EntryError> {
    match store {
        Store::Bucket(_) => {
            let workflows = S3Uri {
                bucket: bucket(entry)?.to_string(),
                key: WORKFLOWS_CONFIG_KEY.to_string(),
                version: None,
            };
            Ok(resolve_workflow(
                remote,
                entry.catalog.as_ref(),
                WorkflowIntent::BucketDefault,
                &workflows,
            )
            .await?)
        }
        Store::Local(_) => Ok(None),
    }
}

/// [`commit_workflow`] for a commit the user asked for (25.14's build, or
/// `commit`), which a workflows config that cannot be read does not refuse:
/// the revision then carries no workflow, and the warning says it went
/// unchecked (#297's warn rulings). A missing session still fails, and so
/// does resolving a workflow against a config that was read, such as a
/// missing schema, as `InstalledPackage::resolve_workflow_for_commit` has it.
async fn commit_workflow_warn(
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
) -> Result<(Option<Workflow>, Option<WorkflowWarning>), EntryError> {
    let Store::Bucket(_) = store else {
        return Ok((None, None));
    };
    let workflows = S3Uri {
        bucket: bucket(entry)?.to_string(),
        key: WORKFLOWS_CONFIG_KEY.to_string(),
        version: None,
    };
    let host = entry.catalog.as_ref();
    let (config, parsed) = match fetch_workflows_config(remote, host, &workflows).await {
        Ok(fetched) => fetched,
        Err(err @ crate::Error::Login(_)) => return Err(err.into()),
        Err(err) => {
            let unchecked = WorkflowWarning::Unchecked {
                bucket: workflows.bucket,
                reason: err.to_string(),
            };
            return Ok((None, Some(unchecked)));
        }
    };
    let workflow = resolve_workflow_from_config(
        remote,
        host,
        WorkflowIntent::BucketDefault,
        config,
        parsed.as_ref(),
    )
    .await?;
    Ok((workflow, None))
}

/// Commits the working folder's changes on the lock's current revision, then
/// pushes the last commit against the base. `None` where there was nothing
/// to commit or push. `known` is a revision's manifest already read, by its
/// hash.
#[allow(clippy::too_many_arguments)]
async fn publish(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    queue: &Queue,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    known: Option<(String, Manifest)>,
    message: &str,
    user_rules: &Path,
) -> Result<Option<Published>, Stop> {
    let key = entry.key();
    let (base, commits) = match lock.packages.get(&key) {
        Some(locked) => (locked.base.clone(), locked.commits.clone()),
        None => (None, Vec::new()),
    };
    let current_hash = commits.last().or(base.as_ref());
    let current = match (current_hash, known) {
        (None, _) => Manifest::default(),
        (Some(hash), Some((known, manifest))) if *hash == known => manifest,
        (Some(hash), _) => manifest_at(storage, remote, store, entry, hash, &commits).await?,
    };

    let folder = &entry.folder;
    if current_hash.is_some() && gone(storage, folder, &current, &entry.paths).await? {
        return Err(EntryError::FolderGone(folder.clone()).into());
    }
    // A package not yet published whose folder is not there has nothing to
    // publish.
    if current_hash.is_none() && !storage.exists(folder).await {
        return Ok(None);
    }
    let contract = lock_file.contract;
    let mut head = current_hash.cloned();
    let committed = commit_folder(
        storage,
        remote,
        store,
        contract,
        entry,
        user_rules,
        (current_hash.map(String::as_str), &current),
        message,
        FolderCheck::AtPush,
    )
    .await?;
    if let Some(FolderCommit {
        hash,
        metadata_text,
        ..
    }) = committed
    {
        crate::failpoint::stop("sync.recorded")?;
        // A package not yet published gets an entry with no base: its
        // first push, which expects no `latest`, establishes one.
        append_commit(lock, &key, &hash);
        lock_file.save(lock).await?;
        if let Err(err) =
            rewrite_metadata(storage, store, entry, &hash, metadata_text.as_deref()).await
        {
            warn!("{METADATA_FILE} was left as it is: {err}");
        }
        head = Some(hash);
    }

    let Some(head) = head.filter(|head| Some(head) != base.as_ref()) else {
        // Committed back to the base: the commits before it are released.
        if let Some(base) = base
            && lock
                .packages
                .get(&key)
                .is_some_and(|locked| !locked.commits.is_empty())
        {
            set_base(lock, lock_file, &key, base).await?;
        }
        return Ok(None);
    };
    let (published, wrote) =
        push_head(storage, remote, store, queue, entry, &head, base.as_deref()).await?;
    if let Published::Advanced { revision, .. } = &published {
        crate::failpoint::stop("sync.pushed")?;
        set_base(lock, lock_file, &key, revision.clone()).await?;
    }
    if wrote {
        after_write(contract, entry, &published).await;
    }
    Ok(Some(published))
}

/// A commit of a working folder, recorded in its store.
pub(crate) struct FolderCommit {
    pub(crate) hash: String,
    /// `metadata.toml` as the commit took it, if there is one.
    pub(crate) metadata_text: Option<Vec<u8>>,
    /// What [`FolderCheck::Warn`] found; empty otherwise.
    pub(crate) warnings: Vec<WorkflowWarning>,
}

/// When [`commit_folder`] checks the commit against the bucket's workflows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderCheck {
    /// Only at push, as the tick commits (#299's sync-keep-blocked): nothing
    /// is checked here, and a config that cannot be read stops the commit.
    AtPush,
    /// Now as well, for a commit the user asked for (#297's warn rulings): a
    /// bucket package is checked against its bucket's current workflows
    /// config and recorded either way, with an `Inconsistent` or `Unchecked`
    /// warning; a local-registry package gets no check and no warning.
    Warn,
}

/// Records what `entry`'s working folder changed from `current`, the lock's
/// current revision and its hash, `None` for a package never published, as
/// a commit on it in `store`, with `message` and the bucket's default
/// workflow. Where the folder changed, the contract's `pre-write` hook runs
/// first, as [`take_changed`] says. `None` where nothing changed, or the
/// commit is the current revision again. `check` says whether the commit is
/// checked against the bucket's workflows now, as [`FolderCheck`] has it.
#[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
pub(crate) async fn commit_folder(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    contract: &Contract,
    entry: &Entry,
    user_rules: &Path,
    (current_hash, current): (Option<&str>, &Manifest),
    message: &str,
    check: FolderCheck,
) -> Result<Option<FolderCommit>, EntryError> {
    let on = current_hash.map(|_| current);
    let (taken, changed) = take_changed(storage, contract, entry, user_rules, current, on).await?;
    if !changed {
        return Ok(None);
    }
    let Taken {
        mut manifest,
        content,
        metadata_text,
    } = taken;
    manifest.header.message = HeaderMessage::Text(message.to_string());
    let unchecked = match check {
        FolderCheck::AtPush => {
            manifest.header.workflow = commit_workflow(remote, store, entry).await?;
            None
        }
        FolderCheck::Warn => {
            let (workflow, unchecked) = commit_workflow_warn(remote, store, entry).await?;
            manifest.header.workflow = workflow;
            unchecked
        }
    };
    let base_hash = current_hash.unwrap_or_default().to_string();
    // `record` checks a `Warn` commit against the bucket the lineage names.
    // Under `AtPush` it names none, so nothing is checked here: the push
    // checks the revision, and stores one the workflow refuses. A
    // local-registry package names none either way.
    let remote_uri = match (check, store) {
        (FolderCheck::Warn, Store::Bucket(_)) => Some(ManifestUri {
            origin: entry.catalog.clone(),
            bucket: bucket(entry)?.to_string(),
            namespace: entry.package.clone(),
            hash: base_hash.clone(),
        }),
        _ => None,
    };
    let lineage = PackageLineage {
        remote_uri,
        base_hash,
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
    let recorded = recorded.with_unchecked(unchecked);
    Ok(recorded.hash().map(|hash| FolderCommit {
        hash: hash.to_string(),
        metadata_text,
        warnings: recorded.warnings.clone(),
    }))
}

/// Appends `hash` to `key`'s commits in `lock`; a package not yet published
/// gets an entry with no base.
pub(crate) fn append_commit(lock: &mut Lock, key: &PackageKey, hash: &str) {
    lock.packages
        .entry(key.clone())
        .or_insert(LockEntry {
            base: None,
            commits: Vec::new(),
        })
        .commits
        .push(hash.to_string());
}

/// What a commit would take from a working folder.
pub(crate) struct Taken {
    /// `current` as the folder has it.
    pub(crate) manifest: Manifest,
    /// The content of each row a file changed or added.
    content: BTreeMap<LogicalKey, RowContent>,
    /// `metadata.toml` as it was read, if there is one.
    metadata_text: Option<Vec<u8>>,
}

/// [`take`]s `entry`'s working folder, and says whether it changed from `on`,
/// the lock's current revision `current` where there is one. Where it did,
/// the contract's `pre-write` hook runs first, and may change the folder, its
/// ignore rules and `metadata.toml` included, so the folder is checked and
/// taken again.
async fn take_changed(
    storage: &(impl Storage + Sync),
    contract: &Contract,
    entry: &Entry,
    user_rules: &Path,
    current: &Manifest,
    on: Option<&Manifest>,
) -> Result<(Taken, bool), EntryError> {
    let policy = contract.ignore_policy(&entry.folder);
    let rules = Rules::load_with(&entry.folder, user_rules, &policy)?;
    let taken = take(storage, entry, &rules, current).await?;
    // The tick's message wins over the metadata file's, so the file's alone
    // is no change.
    let changed = !same_as(&taken.manifest, on, false);
    if !changed || contract.hooks().get(HookPoint::PreWrite).is_none() {
        return Ok((taken, changed));
    }
    hooks::run(contract, HookPoint::PreWrite, entry, None)
        .await
        .map_err(EntryError::from)?;
    if on.is_some() && gone(storage, &entry.folder, current, &entry.paths).await? {
        return Err(EntryError::FolderGone(entry.folder.clone()));
    }
    let rules = Rules::load_with(&entry.folder, user_rules, &policy)?;
    let taken = take(storage, entry, &rules, current).await?;
    let changed = !same_as(&taken.manifest, on, false);
    Ok((taken, changed))
}

/// Takes `entry`'s working folder over `current`, the lock's current
/// revision, with its metadata file applied.
pub(crate) async fn take(
    storage: &(impl Storage + Sync),
    entry: &Entry,
    rules: &Rules,
    current: &Manifest,
) -> Result<Taken, EntryError> {
    let folder = &entry.folder;
    let files = walk(storage, folder, &entry.paths, rules, Some(current)).await?;
    let (mut manifest, content) = compose(storage, current, entry, &files).await?;
    let metadata_text = bytes_at(storage, &folder.join(METADATA_FILE)).await?;
    metadata_file(storage, folder)
        .await?
        .apply(&mut manifest, &entry.paths);
    Ok(Taken {
        manifest,
        content,
        metadata_text,
    })
}

/// Writes the metadata file of the commit `hash` into `entry`'s folder,
/// where it still holds `planned_on`: the folder holds the commit now, its
/// message included, so a later read does not take the file for a local
/// change.
pub(crate) async fn rewrite_metadata(
    storage: &(impl Storage + Sync),
    store: &Store,
    entry: &Entry,
    hash: &str,
    planned_on: Option<&[u8]>,
) -> Result<(), EntryError> {
    let committed =
        Manifest::from_path(storage, &store.installed_manifest(&entry.package, hash)).await?;
    let text = render(&committed).map_err(EntryError::MetadataUnwritable)?;
    write_metadata(storage, &entry.folder, text, planned_on).await
}

/// Pushes the local commit `head` against `base`, squashing the commits
/// before it. A revision the bucket's workflow refuses is still stored, and
/// one stored without becoming `latest` is kept in `queue`; a later push of
/// it that the bucket still holds is not uploaded again. Also says whether
/// the push wrote to the bucket: stored the revision, or moved `latest`.
/// In the local registry, see [`push_local`].
async fn push_head(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    queue: &Queue,
    entry: &Entry,
    head: &str,
    base: Option<&str>,
) -> Result<(Published, bool), EntryError> {
    if let Store::Local(local) = store {
        return push_local(storage, local, queue, entry, head, base).await;
    }
    let head_only = [head.to_string()];
    let manifest = manifest_at(storage, remote, store, entry, head, &head_only).await?;
    push_manifest(storage, remote, queue, entry, head, manifest, base).await
}

/// [`push_head`], of `manifest`, the revision `head`.
pub(crate) async fn push_manifest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    queue: &Queue,
    entry: &Entry,
    head: &str,
    manifest: Manifest,
    base: Option<&str>,
) -> Result<(Published, bool), EntryError> {
    let target = PushTarget {
        handle: handle(entry)?,
        origin: entry.catalog.clone(),
        parent: base.map(str::to_string),
    };
    let folder = folder_key(entry);
    let kept = queue.kept(&folder).await?;
    let keep = |hash: &str, reason| {
        let revision = KeptRevision {
            hash: hash.to_string(),
            base: base.map(str::to_string),
            reason,
        };
        queue.keep(&folder, revision)
    };
    // A config that cannot be read or applied fails the push before anything
    // is uploaded; only the workflow's refusal is stored and kept.
    let refused = match check_workflow(remote, &target, &manifest).await {
        Ok(()) => None,
        Err(crate::Error::WorkflowValidation(err @ WorkflowValidationError::Rejected(_))) => {
            Some(err.to_string())
        }
        Err(err) => return Err(err.into()),
    };
    let uri = revision(entry, head)?;
    let in_bucket = kept
        .as_ref()
        .is_some_and(|kept| kept.hash == head && kept.base.as_deref() == base)
        && remote
            .exists(entry.catalog.as_ref(), &S3Uri::from(uri.clone()))
            .await?;
    let stored = if in_bucket {
        uri
    } else {
        Box::pin(store_files(
            storage,
            remote,
            &target,
            manifest,
            &BTreeMap::new(),
        ))
        .await?
    };
    if let Some(reason) = refused {
        keep(&stored.hash, KeptReason::WorkflowBlocked).await?;
        let blocked = Published::WorkflowBlocked {
            revision: stored.hash,
            reason,
        };
        return Ok((blocked, !in_bucket));
    }
    let pushed = tag_stored(remote, &target, stored).await;
    let published = settle(remote, &target, pushed).await?;
    if let Published::Stored { revision, .. } = &published {
        keep(revision, KeptReason::LatestMoved).await?;
    }
    let wrote = !in_bucket || matches!(published, Published::Advanced { .. });
    Ok((published, wrote))
}

/// [`push_head`] to the local registry `local`, by a caller holding the
/// package's lock: the commit `head` is published there against `base`, with
/// no workflow to check. One stored without becoming `latest` is kept in
/// `queue`, as a bucket's is.
async fn push_local(
    storage: &(impl Storage + Sync),
    local: &LocalRegistry,
    queue: &Queue,
    entry: &Entry,
    head: &str,
    base: Option<&str>,
) -> Result<(Published, bool), EntryError> {
    let in_registry = storage.exists(local.revision(head)).await;
    let published = match local
        .publish_held(storage, &entry.package, base, head)
        .await?
    {
        LocalPublish::Advanced => Published::Advanced {
            revision: head.to_string(),
            latest_unchecked: false,
        },
        LocalPublish::Diverged(latest) => {
            let kept = KeptRevision {
                hash: head.to_string(),
                base: base.map(str::to_string),
                reason: KeptReason::LatestMoved,
            };
            queue.keep(&folder_key(entry), kept).await?;
            Published::Stored {
                revision: head.to_string(),
                latest,
            }
        }
    };
    let wrote = !in_registry || matches!(published, Published::Advanced { .. });
    Ok((published, wrote))
}

/// The key `queue.db` keeps `entry`'s working folder under.
pub(crate) fn folder_key(entry: &Entry) -> FolderKey {
    FolderKey {
        registry: Some(entry.registry.clone()),
        package: entry.package.clone(),
        folder: entry.folder.clone(),
    }
}

pub(crate) fn revision(entry: &Entry, hash: &str) -> Result<ManifestUri, EntryError> {
    Ok(ManifestUri {
        origin: entry.catalog.clone(),
        bucket: bucket(entry)?.to_string(),
        namespace: entry.package.clone(),
        hash: hash.to_string(),
    })
}

/// The manifest of `hash`, the lock's current revision of `entry`: the last
/// of `commits`, from `store`, or else the base. A commit the store cleared
/// once the bucket held it, as one stored without becoming `latest`, is read
/// from the bucket.
pub(crate) async fn manifest_at(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    hash: &str,
    commits: &[String],
) -> Result<Manifest, EntryError> {
    let installed = store.installed_manifest(&entry.package, hash);
    if commits.last().is_some_and(|head| head == hash) && storage.exists(&installed).await {
        Ok(Manifest::from_path(storage, &installed).await?)
    } else {
        base_manifest(storage, remote, store, entry, hash).await
    }
}

/// Puts every file of `current`, the revision `hash`, under `entry`'s paths
/// back into its working folder, which is missing or empty, and writes the
/// metadata file where there is none. A file a local commit changed is
/// copied from `store`, the rest fetched from the bucket. Nothing is placed
/// unless every file can be: a folder left partly filled would publish the
/// rest as deleted.
async fn read_back(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    hash: &str,
    current: &Manifest,
) -> Result<ReadReport, EntryError> {
    let folder = &entry.folder;
    let rows = covered_rows(entry, current);
    let staging = folder.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let read = async {
        let mut report = stage(storage, remote, store, entry, hash, current, &staging).await?;
        if !report.complete() {
            return Ok(report);
        }
        let text = render(current).map_err(EntryError::MetadataUnwritable)?;
        let keys = || rows.iter().map(|row| &row.logical_key);
        refuse_linked_ancestors(storage, folder, keys()).await?;
        let placed = async {
            for key in keys() {
                let path = key.join_to(folder);
                // A rename replaces what it finds.
                if storage.exists(&path).await {
                    return Err(EntryError::Changed(key.to_path_buf()));
                }
                if let Some(parent) = path.parent() {
                    storage.create_dir_all(parent).await?;
                }
                storage.rename(key.join_to(&staging), &path).await?;
                report.placed.push(key.to_path_buf());
                crate::failpoint::stop("sync.read_back")?;
            }
            match bytes_at(storage, &folder.join(METADATA_FILE)).await? {
                None => {
                    write_metadata(storage, folder, text, None).await?;
                    report.placed.push(PathBuf::from(METADATA_FILE));
                }
                Some(bytes) if bytes != text.as_bytes() => {
                    report.kept.push(PathBuf::from(METADATA_FILE));
                }
                Some(_) => {}
            }
            Ok(())
        }
        .await;
        if let Err(err) = placed {
            // Back to empty, so the next tick reads it back again.
            for done in &report.placed {
                let _ = storage.remove_file(folder.join(done)).await;
            }
            return Err(err);
        }
        Ok::<_, EntryError>(report)
    }
    .await;
    let _ = storage.remove_dir_all(&staging).await;
    read
}

/// The rows of `manifest` under `entry`'s paths.
pub(crate) fn covered_rows<'a>(entry: &Entry, manifest: &'a Manifest) -> Vec<&'a ManifestRow> {
    manifest
        .rows
        .iter()
        .filter(|row| entry.paths.covers(&row.logical_key))
        .collect()
}

/// Puts every file of `manifest`, the revision `hash`, under `entry`'s paths
/// into `staging`, each at its logical key: a file a local commit changed is
/// copied from `store` and checked against its row, the rest fetched from
/// the bucket. The report names what could not be staged, and stops at a
/// copy that does not hold its row.
pub(crate) async fn stage(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    hash: &str,
    manifest: &Manifest,
    staging: &Path,
) -> Result<ReadReport, EntryError> {
    let (fetched, copied): (Vec<&ManifestRow>, Vec<&ManifestRow>) = covered_rows(entry, manifest)
        .into_iter()
        .partition(|row| row.physical_key.starts_with("s3://"));
    let installed = install_to(
        storage,
        remote,
        entry.catalog.as_ref(),
        &Manifest {
            rows: fetched.into_iter().cloned().collect(),
            ..Manifest::default()
        },
        staging,
        None,
    )
    .await?;
    let mut report = ReadReport {
        revision: hash.to_string(),
        skipped: installed.skipped,
        hash_mismatch: installed.hash_mismatch,
        ..ReadReport::default()
    };
    if !report.complete() {
        return Ok(report);
    }
    for row in copied {
        let staged = row.logical_key.join_to(staging);
        if let Some(parent) = staged.parent() {
            storage.create_dir_all(parent).await?;
        }
        storage.copy(store.object_path(&row.hash), &staged).await?;
        if !OnDisk::read(storage, &staged, row.hash.algorithm())
            .await?
            .holds(row)
        {
            report.hash_mismatch.push(row.logical_key.to_path_buf());
            return Ok(report);
        }
    }
    Ok(report)
}

/// The manifest of `hash`, a revision of `entry`'s package in its bucket,
/// kept in `store` after its first fetch; or one the local registry holds.
pub(crate) async fn base_manifest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    hash: &str,
) -> Result<Manifest, EntryError> {
    match store {
        Store::Bucket(store) => {
            let uri = revision(entry, hash)?;
            Ok(cache_manifest_at(storage, remote, &uri, &store.cached_manifest(hash)).await?)
        }
        Store::Local(local) => Ok(Manifest::from_path(storage, &local.revision(hash)).await?),
    }
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
            // A link where the file, or a folder above it, was is not its
            // deletion: the row stays.
            if linked(storage, &entry.folder, &row.logical_key).await {
                rows.push(row.clone());
            }
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
    // A folder whose files were all deleted, or that turned into a file,
    // loses its metadata; one that held none keeps it, until a file takes its
    // name.
    let folders = |rows: &[ManifestRow]| -> BTreeSet<String> {
        rows.iter()
            .filter_map(|row| row.logical_key.to_str())
            .flat_map(|key| {
                key.match_indices('/')
                    .map(move |(at, _)| key[..at].to_string())
            })
            .collect()
    };
    let (before, after) = (folders(&current.rows), folders(&rows));
    let files: BTreeSet<&LogicalKey> = rows.iter().map(|row| &row.logical_key).collect();
    let mut dir_meta = current.dir_meta.clone();
    dir_meta.retain(|dir| {
        !files.contains(&dir.logical_key)
            && dir
                .logical_key
                .to_str()
                .is_none_or(|key| !before.contains(key) || after.contains(key))
    });
    let manifest = Manifest {
        header: current.header.clone(),
        rows,
        dir_meta,
    };
    Ok((manifest, content))
}

/// Whether a link stands at `key` in `folder`, or at a folder between them.
async fn linked(storage: &(impl Storage + Sync), folder: &Path, key: &LogicalKey) -> bool {
    let path = key.join_to(folder);
    for place in path
        .ancestors()
        .take_while(|place| place.starts_with(folder) && *place != folder)
    {
        if storage.is_symlink(place).await {
            return true;
        }
    }
    false
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

mod merge;

#[cfg(test)]
pub(crate) mod tests;
