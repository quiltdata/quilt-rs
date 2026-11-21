//! `status`: each entry of a dataspace against its lock, its registry and
//! its working folder, read and never written: the four hashes and their
//! verdict (25.6), the folder's changes since the lock's current revision,
//! the package's rows in `conflict.json`, and the revision `queue.db` kept.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;

use jiff::Timestamp;
use thiserror::Error;

use crate::LogicalKey;
use crate::dataspace::EntryError;
use crate::dataspace::conflict::ConflictMap;
use crate::dataspace::conflict::Row;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Paths;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read::gone;
use crate::dataspace::store::Store;
use crate::dataspace::sync::folder_key;
use crate::dataspace::sync::latest;
use crate::dataspace::sync::take;
use crate::domain::DomainRoot;
use crate::domain::LocalNames;
use crate::io::manifest::fetch_jsonl;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::Change;
use crate::lineage::ChangeSet;
use crate::lineage::PackageState;
use crate::lineage::UpstreamState;
use crate::lineage::WorkingCopy;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::queue::KeptRevision;
use crate::queue::Queue;
use crate::quiltignore::Rules;
use quilt_uri::ManifestUri;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;

/// The four hashes a package's state is computed from (25.6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hashes {
    /// The lock's base: the revision `latest` named when the package was
    /// last read or published here. `None` for a package never published,
    /// and for one the lock does not name.
    pub base: Option<String>,
    /// The lock's last unpushed commit, else its base.
    pub current: Option<String>,
    /// What `latest` names, as far as [`EntryStatus::latest_source`] says;
    /// `None` where it names nothing, or nothing is known.
    pub latest: Option<String>,
    /// The revision the folder was read from: the entry's pinned revision,
    /// else the base.
    pub remote: Option<String>,
}

/// Where the `latest` a status compared against came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatestSource {
    /// The registry, asked just now: the bucket, or the local registry's
    /// own tag.
    Registry,
    /// The Domain's latest index for the bucket: the last hash it was known
    /// to name, not asked now.
    Index,
    /// The lock's base, which `latest` named when the package was last read
    /// or published here.
    Lock,
    /// Nothing could say.
    Unknown,
}

/// One entry of a dataspace, as [`status`] found it.
#[derive(Debug)]
#[non_exhaustive]
pub struct EntryStatus {
    pub entry: Entry,
    pub hashes: Hashes,
    pub latest_source: LatestSource,
    /// The verdict over [`Self::hashes`]: [`UpstreamState::Local`] for a
    /// package nothing was published for yet, and [`UpstreamState::Error`]
    /// where `latest` is unknown.
    pub upstream_state: UpstreamState,
    /// Whether the lock names the package. One it does not name is not
    /// installed: its folder was never read whole.
    pub installed: bool,
    /// The lock's local commits not pushed yet, oldest first.
    pub unpushed: Vec<String>,
    /// What a commit of the working folder would change, under the entry's
    /// paths and the dataspace's ignore policy, against the lock's current
    /// revision. A missing file is listed as removed only under `sync` and
    /// `replace`, where it is a deletion: `read` fills it, and `write` never
    /// deletes.
    pub changes: ChangeSet,
    /// Whether the metadata file changes the package's or a file's
    /// metadata.
    pub metadata_changed: bool,
    /// [`WorkingCopy::Unavailable`] where the lock names a revision and the
    /// folder is missing or holds none of its files: nothing was compared,
    /// and the next sync reads it back.
    pub working_copy: WorkingCopy,
    /// `false` where the folder could not be compared, so `changes` is empty
    /// for want of looking and [`EntryStatus::state`] is unknown.
    pub compared: bool,
    /// The package's rows in `conflict.json`, by logical key.
    pub conflicts: BTreeMap<LogicalKey, Row>,
    /// The revision `queue.db` kept for the package's folder: stored in its
    /// registry, and not made `latest`.
    pub kept: Option<KeptRevision>,
    /// What could not be read; the fields it would have filled are left
    /// empty.
    pub problems: Vec<EntryError>,
    /// The folders inside the working folder that the comparison, as the
    /// tick's commit, left out whatever any rule says, and why.
    pub left_out: Vec<LeftOut>,
}

/// A folder inside an entry's working folder that is never packaged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// Its path below the working folder.
    pub path: PathBuf,
    pub reason: LeftOutReason,
}

/// Why a [`LeftOut`] folder is never packaged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LeftOutReason {
    /// It holds a `data.toml` of its own (25o item 7).
    #[error("it holds a {CONTRACT_FILE} of its own, so it is another dataspace")]
    Dataspace,
}

impl EntryStatus {
    /// The state a front end shows, resolved as [`PackageState::resolve`]
    /// resolves a ref's.
    #[must_use]
    pub fn state(&self) -> PackageState {
        if matches!(self.working_copy, WorkingCopy::Unavailable(_)) {
            return PackageState::Unavailable;
        }
        if !self.compared {
            return PackageState::Unknown;
        }
        let pending = !self.unpushed.is_empty() && self.hashes.latest != self.hashes.current;
        let files = self.changes.len() + usize::from(self.metadata_changed);
        PackageState::resolve(self.upstream_state, pending, true, Some(files))
    }
}

/// What [`status`] found.
#[derive(Debug)]
#[non_exhaustive]
pub struct DataspaceStatus {
    /// Each entry, in the contract's order.
    pub entries: Vec<EntryStatus>,
    /// Whether there is a lock.
    pub locked: bool,
    /// When the dataspace was first filled; `None` until then.
    pub filled: Option<Timestamp>,
    /// What in the lock's header is not this dataspace's or the writer's:
    /// a tool does not proceed on the lock until it is fixed.
    pub mismatches: Vec<Mismatch>,
    /// What could not be read for the whole dataspace, such as
    /// `conflict.json`.
    pub problems: Vec<EntryError>,
}

/// Why a dataspace's status could not be read at all.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StatusError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),
}

impl From<ContractError> for StatusError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for StatusError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

/// The status of each entry of the dataspace in the folder `dataspace`. It
/// writes nothing: not the contract, the lock, `conflict.json`, the working
/// folders, `queue.db` or the Domain, whose local registry it does not even
/// name.
///
/// The hashes come from the lock and the registry: `latest` is asked of
/// the bucket where `fetch`, falling back to `domain`'s latest index and
/// then to the lock's base where the bucket cannot be reached; without
/// `fetch` the index answers, else the lock's base. A local registry's
/// `latest` is its own tag, always read. Their verdict is today's: a
/// package with no base and local commits is [`UpstreamState::Local`] while
/// nothing is published, up to date once `latest` names its last commit,
/// and diverged where `latest` names anything else (entry 25m); a base whose
/// `latest` is gone is diverged, as the tick has it. A pinned `read` entry
/// is compared with its pin rather than `latest`.
///
/// The working folder is compared as the tick's commit would take it, with
/// the contract's ignore policy and the user-wide rules in `user_rules`.
/// A revision's manifest is read from `domain`'s store, or else from the
/// bucket, and is not cached. An entry of `domain`'s local registry, under
/// its name or one it had, is read from it; with no Domain, or a local
/// registry never named, such an entry has a problem and no verdict. The
/// revision kept for each folder is read from `queue`, where one is given.
///
/// The lock is read whatever its header says; [`DataspaceStatus::mismatches`]
/// says what in it is not `writer`'s or this dataspace's.
#[allow(clippy::too_many_arguments)]
pub async fn status(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    queue: Option<&Queue>,
    dataspace: &Path,
    writer: &Writer,
    user_rules: &Path,
    fetch: bool,
) -> Result<DataspaceStatus, StatusError> {
    status_of(
        storage, remote, domain, queue, dataspace, writer, user_rules, fetch, None,
    )
    .await
}

/// The status of the one entry of the dataspace in the folder `dataspace`
/// equal to `wanted`, read as [`status`] reads each, and no other: no other
/// folder is walked and no other registry asked. `None` where the contract
/// has no such entry.
#[allow(clippy::too_many_arguments)]
pub async fn entry_status(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    queue: Option<&Queue>,
    dataspace: &Path,
    writer: &Writer,
    user_rules: &Path,
    wanted: &Entry,
    fetch: bool,
) -> Result<Option<EntryStatus>, StatusError> {
    let status = status_of(
        storage,
        remote,
        domain,
        queue,
        dataspace,
        writer,
        user_rules,
        fetch,
        Some(wanted),
    )
    .await?;
    Ok(status.entries.into_iter().next())
}

#[allow(clippy::too_many_arguments)]
async fn status_of(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    queue: Option<&Queue>,
    dataspace: &Path,
    writer: &Writer,
    user_rules: &Path,
    fetch: bool,
    only: Option<&Entry>,
) -> Result<DataspaceStatus, StatusError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let mut lock = Lock::read(storage, &contract).await?;
    let names = match domain {
        Some(domain) => domain.local().read_names(storage).await.ok().flatten(),
        None => None,
    };
    let mut problems = Vec::new();
    let mut conflicts = ConflictMap::read(storage, contract.dataspace())
        .await
        .unwrap_or_else(|err| {
            problems.push(err);
            ConflictMap::default()
        });
    if let Some(names) = &names {
        contract.name_local(names)?;
        if let Some(lock) = &mut lock {
            lock.name_local(names);
        }
        conflicts.name_local(names);
    }
    let mismatches = lock
        .as_ref()
        .map_or_else(Vec::new, |lock| lock.mismatches(&contract, writer));

    let mut entries = Vec::new();
    for entry in contract.entries() {
        if only.is_some_and(|wanted| wanted != entry) {
            continue;
        }
        let locked = lock
            .as_ref()
            .and_then(|lock| lock.packages.get(&entry.key()));
        let mut status = Reading {
            storage,
            remote,
            domain,
            names: names.as_ref(),
            contract: &contract,
            user_rules,
            fetch,
        }
        .entry(entry, locked)
        .await;
        status.conflicts = conflicts
            .packages
            .get(&entry.key())
            .cloned()
            .unwrap_or_default();
        if let Some(queue) = queue {
            match queue.kept(&folder_key(entry)).await {
                Ok(kept) => status.kept = kept,
                Err(err) => status.problems.push(err.into()),
            }
        }
        entries.push(status);
    }
    Ok(DataspaceStatus {
        entries,
        locked: lock.is_some(),
        filled: lock.as_ref().and_then(|lock| lock.header.filled),
        mismatches,
        problems,
    })
}

/// What reading one entry's status needs.
struct Reading<'a, S, R> {
    storage: &'a S,
    remote: &'a R,
    domain: Option<&'a DomainRoot>,
    names: Option<&'a LocalNames>,
    contract: &'a Contract,
    user_rules: &'a Path,
    fetch: bool,
}

impl<S: Storage + Sync, R: Remote> Reading<'_, S, R> {
    async fn entry(&self, entry: &Entry, locked: Option<&LockEntry>) -> EntryStatus {
        let (base, unpushed) = match locked {
            Some(locked) => (locked.base.clone(), locked.commits.clone()),
            None => (None, Vec::new()),
        };
        let current = unpushed.last().or(base.as_ref()).cloned();
        let remote = match &entry.revision {
            RevisionPointer::Hash(hash) | RevisionPointer::HashPrefix(hash) => Some(hash.clone()),
            RevisionPointer::Tag(_) => base.clone(),
        };
        let mut status = EntryStatus {
            entry: entry.clone(),
            hashes: Hashes {
                base,
                current,
                latest: None,
                remote,
            },
            latest_source: LatestSource::Unknown,
            upstream_state: UpstreamState::Error,
            installed: locked.is_some(),
            unpushed,
            changes: ChangeSet::new(),
            metadata_changed: false,
            working_copy: WorkingCopy::Available,
            compared: true,
            conflicts: BTreeMap::new(),
            kept: None,
            problems: Vec::new(),
            left_out: Vec::new(),
        };
        let store = match self.store(entry) {
            Ok(store) => store,
            Err(err) => {
                status.problems.push(err);
                return status;
            }
        };
        let (latest, source) = self
            .latest(store.as_ref(), entry, status.hashes.base.as_deref())
            .await;
        status.hashes.latest = latest;
        status.latest_source = source;
        if source != LatestSource::Unknown {
            status.upstream_state = verdict(&status.hashes, &status.unpushed, &entry.revision);
        }
        if let Err(err) = self.compare(store.as_ref(), entry, &mut status).await {
            status.compared = false;
            status.problems.push(err);
        }
        status
    }

    /// The store `entry`'s package is kept in, read only: `None` for a
    /// bucket's entry with no Domain.
    fn store(&self, entry: &Entry) -> Result<Option<Store>, EntryError> {
        match &entry.registry {
            Registry::S3(bucket) => match self.domain {
                Some(domain) => Ok(Some(Store::Bucket(domain.registry(bucket)?))),
                None => Ok(None),
            },
            Registry::Local(name) => match (self.domain, self.names) {
                (Some(domain), Some(names)) if names.answers_to(name) => {
                    Ok(Some(Store::Local(domain.local())))
                }
                _ => Err(EntryError::LocalRegistry),
            },
        }
    }

    /// What `latest` names for `entry`, and where that came from.
    async fn latest(
        &self,
        store: Option<&Store>,
        entry: &Entry,
        base: Option<&str>,
    ) -> (Option<String>, LatestSource) {
        if let Some(Store::Local(local)) = store {
            return match local.read_latest(self.storage, &entry.package).await {
                Ok(latest) => (latest, LatestSource::Registry),
                Err(_) => (None, LatestSource::Unknown),
            };
        }
        if self.fetch
            && let Ok(latest) = latest(self.remote, entry).await
        {
            return (latest, LatestSource::Registry);
        }
        if let Some(Store::Bucket(store)) = store
            && let Ok(Some(latest)) = store.read_latest(self.storage, &entry.package).await
        {
            return (Some(latest), LatestSource::Index);
        }
        match base {
            Some(base) => (Some(base.to_string()), LatestSource::Lock),
            None => (None, LatestSource::Unknown),
        }
    }

    /// Compares `entry`'s working folder with the lock's current revision,
    /// as the tick's commit would take it.
    async fn compare(
        &self,
        store: Option<&Store>,
        entry: &Entry,
        status: &mut EntryStatus,
    ) -> Result<(), EntryError> {
        let folder = &entry.folder;
        let current = match &status.hashes.current {
            Some(hash) => Some(self.manifest(store, entry, hash).await?),
            None => None,
        };
        if let Some(current) = &current
            && gone(self.storage, folder, current, &entry.paths).await?
        {
            status.working_copy = WorkingCopy::Unavailable(folder.clone());
            return Ok(());
        }
        if current.is_none() && !self.storage.exists(folder).await {
            return Ok(());
        }
        let current = current.unwrap_or_default();
        let policy = self.contract.ignore_policy(folder);
        let rules = Rules::load_with(folder, self.user_rules, &policy)?;
        let taken = take(self.storage, entry, &rules, &current).await?.manifest;
        status.left_out = nested_dataspaces(self.storage, folder, &rules).await?;
        let deletes = matches!(entry.access, Access::Sync | Access::Replace);
        status.changes = changes(&current, &taken, &entry.paths, deletes);
        let was: std::collections::HashMap<_, _> = current
            .rows
            .iter()
            .map(|row| (&row.logical_key, row))
            .collect();
        status.metadata_changed = !taken.header.same_user_meta(&current.header)
            || taken.dir_meta != current.dir_meta
            || taken.rows.iter().any(|row| {
                was.get(&row.logical_key)
                    .is_some_and(|was| !row.same_meta(was))
            });
        Ok(())
    }

    /// The manifest of the revision `hash` of `entry`'s package: from the
    /// store, where it holds one, else from the bucket. Nothing is cached.
    async fn manifest(
        &self,
        store: Option<&Store>,
        entry: &Entry,
        hash: &str,
    ) -> Result<Manifest, EntryError> {
        manifest_in(self.storage, self.remote, store, entry, hash).await
    }
}

/// The manifest of the revision `hash` of `entry`'s package, as [`status`]
/// reads it: from `domain`'s store for the package, where it holds one, else
/// from the bucket. Nothing is cached. A front end reads a revision's rows
/// with it, as the file list of a package's page.
pub async fn revision_manifest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    entry: &Entry,
    hash: &str,
) -> Result<Manifest, EntryError> {
    let names = match domain {
        Some(domain) => domain.local().read_names(storage).await.ok().flatten(),
        None => None,
    };
    let store = match (&entry.registry, domain) {
        (Registry::S3(bucket), Some(domain)) => Some(Store::Bucket(domain.registry(bucket)?)),
        (Registry::Local(name), Some(domain))
            if names.as_ref().is_some_and(|names| names.answers_to(name)) =>
        {
            Some(Store::Local(domain.local()))
        }
        (Registry::Local(_), _) => return Err(EntryError::LocalRegistry),
        (_, None) => None,
    };
    manifest_in(storage, remote, store.as_ref(), entry, hash).await
}

async fn manifest_in(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: Option<&Store>,
    entry: &Entry,
    hash: &str,
) -> Result<Manifest, EntryError> {
    match store {
        Some(Store::Local(local)) => {
            let installed = local.installed_manifest(&entry.package, hash);
            let path = if storage.exists(&installed).await {
                installed
            } else {
                local.revision(hash)
            };
            return Ok(Manifest::from_path(storage, &path).await?);
        }
        Some(Store::Bucket(store)) => {
            let installed = store.installed_manifest(&entry.package, hash);
            for path in [installed, store.cached_manifest(hash)] {
                if storage.exists(&path).await {
                    return Ok(Manifest::from_path(storage, &path).await?);
                }
            }
        }
        None => {}
    }
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry);
    };
    let uri = ManifestUri {
        origin: entry.catalog.clone(),
        bucket: bucket.clone(),
        namespace: entry.package.clone(),
        hash: hash.to_string(),
    };
    Ok(fetch_jsonl(remote, &uri).await?)
}

/// The verdict over `hashes`, with `unpushed` the lock's commits; see
/// [`status`].
fn verdict(hashes: &Hashes, unpushed: &[String], revision: &RevisionPointer) -> UpstreamState {
    let target = match revision {
        RevisionPointer::Hash(_) | RevisionPointer::HashPrefix(_) => hashes.remote.as_deref(),
        RevisionPointer::Tag(_) => hashes.latest.as_deref(),
    };
    let base = hashes.base.as_deref();
    let current = hashes.current.as_deref();
    // A pin by prefix names the base where the base starts with it; a pin
    // by time names whatever was read, which only a registry could resolve.
    let target = match (revision, base) {
        (RevisionPointer::HashPrefix(prefix), Some(base)) if base.starts_with(prefix.as_str()) => {
            Some(base)
        }
        (RevisionPointer::Tag(Tag::Timestamp(_)), Some(base)) => Some(base),
        _ => target,
    };
    match (base, current) {
        // Not in the lock: there is a revision to read, or nothing at all.
        (None, None) if target.is_some() => UpstreamState::Behind,
        (None, None) => UpstreamState::Local,
        // Commits never published (entry 25m).
        (None, Some(head)) => match target {
            None => UpstreamState::Local,
            Some(target) if target == head => UpstreamState::UpToDate,
            Some(_) => UpstreamState::Diverged,
        },
        (Some(base), current) => {
            let current = current.unwrap_or(base);
            match target {
                // The push landed and the lock was not written after it.
                Some(target) if !unpushed.is_empty() && target == current => {
                    UpstreamState::UpToDate
                }
                // `latest` is gone from under the base.
                None => UpstreamState::Diverged,
                Some(target) => match (current != base, target != base) {
                    (false, false) => UpstreamState::UpToDate,
                    (false, true) => UpstreamState::Behind,
                    (true, false) => UpstreamState::Ahead,
                    (true, true) => UpstreamState::Diverged,
                },
            }
        }
    }
}

/// The files `taken`, the folder as a commit would take it, changes from
/// `current` under `paths`; a file `current` has and `taken` lacks only
/// where `deletes`.
fn changes(current: &Manifest, taken: &Manifest, paths: &Paths, deletes: bool) -> ChangeSet {
    let covered = |manifest: &Manifest| -> BTreeMap<LogicalKey, ManifestRow> {
        manifest
            .rows
            .iter()
            .filter(|row| paths.covers(&row.logical_key))
            .map(|row| (row.logical_key.clone(), row.clone()))
            .collect()
    };
    let (before, mut after) = (covered(current), covered(taken));
    let mut changes = ChangeSet::new();
    for (key, was) in before {
        let path = key.as_path().to_path_buf();
        match after.remove(&key) {
            Some(row) if !row.matches_content(&was) => {
                changes.insert(path, Change::Modified(row));
            }
            None if deletes => {
                changes.insert(path, Change::Removed(was));
            }
            // The same bytes, or a missing file that is no deletion.
            Some(_) | None => {}
        }
    }
    for (key, row) in after {
        changes.insert(key.as_path().to_path_buf(), Change::Added(row));
    }
    changes
}

/// The folders inside `folder` that hold a `data.toml` of their own, which
/// the tick's commit leaves out as other dataspaces, each by its path below
/// `folder`. A folder `rules` leave out is not looked in.
async fn nested_dataspaces(
    storage: &(impl Storage + Sync),
    folder: &Path,
    rules: &Rules,
) -> Result<Vec<LeftOut>, EntryError> {
    let mut found = Vec::new();
    let mut queue = VecDeque::from([PathBuf::new()]);
    while let Some(prefix) = queue.pop_front() {
        let Ok(mut entries) = storage.read_dir(&folder.join(&prefix)).await else {
            continue;
        };
        // A folder that goes away mid-walk is left out of a list that only informs.
        while let Ok(Some(entry)) = entries.next_entry().await {
            if !entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let relative = prefix.join(entry.file_name());
            if storage.is_file(entry.path().join(CONTRACT_FILE)).await {
                found.push(LeftOut {
                    path: relative,
                    reason: LeftOutReason::Dataspace,
                });
            } else if rules.dir_ignored_by(&relative, false).is_none() {
                queue.push_back(relative);
            }
        }
    }
    found.sort_by(|one, other| one.path.cmp(&other.path));
    Ok(found)
}

#[cfg(test)]
pub(crate) mod tests;
