//! Autosync's runner: the flow a host calls on each tick. For each dataspace
//! on autosync it reads the contract and the lock, queues an entry's job
//! where the revision it follows is not the lock's base, and runs each job
//! that is not waiting out a failure, by its access: `read` reads, `write`
//! and `replace` publish, and `sync` runs the tick. Jobs, failures and kept
//! revisions are kept in the [`Queue`], so they outlast the process.

use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use thiserror::Error;
use tokio::sync::Mutex;

use crate::dataspace::EntryError;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Writer;
use crate::dataspace::publish::Published;
use crate::dataspace::publish_all::EntryPublish;
use crate::dataspace::publish_all::PublishAllError;
use crate::dataspace::publish_all::publish_some;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read_all::EntryRead;
use crate::dataspace::read_all::ReadAllError;
use crate::dataspace::read_all::read_some;
use crate::dataspace::sync::EntrySync;
use crate::dataspace::sync::SyncError;
use crate::dataspace::sync::folder_key;
use crate::dataspace::sync::latest;
use crate::dataspace::sync::sync;
use crate::domain::DomainRoot;
use crate::io::manifest::resolve_manifest_uri;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::queue::JobKey;
use crate::queue::KeptReason;
use crate::queue::KeptRevision;
use crate::queue::Queue;
use crate::queue::QueueError;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageUri;
use quilt_uri::Tag;

/// What a [`Runner`] runs its jobs with.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The Domain whose stores the `sync` tick commits into.
    pub domain: DomainRoot,
    /// Who writes the locks.
    pub writer: Writer,
    /// The user-wide ignore rules file, which need not exist.
    pub user_rules: PathBuf,
    /// The message of each revision the `sync` tick commits.
    pub message: String,
}

/// Runs autosync's jobs from the queue it owns.
pub struct Runner {
    queue: Queue,
    settings: Settings,
    /// One tick at a time, so a job this queue started is never another
    /// tick's.
    ticking: Mutex<()>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// What one [`Runner::tick`] did, dataspace by dataspace, in the order
/// autosync was turned on.
#[derive(Debug)]
pub struct Tick {
    pub dataspaces: Vec<DataspaceTick>,
}

/// What one tick did in one dataspace.
#[derive(Debug)]
#[non_exhaustive]
pub struct DataspaceTick {
    pub dataspace: PathBuf,
    /// Each entry of the contract, in its order, with what the tick did.
    pub entries: Vec<(Entry, EntryTick)>,
    /// The kept revisions released, `latest` naming them or the lock's base
    /// having moved off theirs.
    pub released: Vec<(Entry, KeptRevision)>,
    /// Why the contract or lock could not be read, so nothing was run, or why
    /// the run of one access stopped.
    pub errors: Vec<DataspaceError>,
}

/// What one tick did with one entry.
#[derive(Debug)]
#[non_exhaustive]
pub enum EntryTick {
    /// Nothing ran: no job was queued, the revision it follows being the
    /// lock's base.
    Idle,
    /// The runner does not run it.
    Skipped(Skip),
    /// The revision it follows could not be looked up, so no job was queued
    /// or run; the next tick looks again.
    Unobserved(EntryError),
    /// Its job waits out a failure until `retry_at`.
    Waiting {
        retry_at: SystemTime,
    },
    Read(EntryRead),
    Published(EntryPublish),
    Synced(EntrySync),
    /// The run of its access stopped for the whole dataspace; see
    /// [`DataspaceTick::errors`]. Its job counts a failure.
    Stopped,
}

/// Why the runner leaves an entry alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Skip {
    /// A local registry's package, which no domainless operation reaches yet.
    LocalRegistry,
    /// A `write` or `replace` entry whose package is also under `read`.
    /// Whose metadata a publish of such a package keeps is not ruled yet, so
    /// it is not published unattended; its `read` entry is still read.
    ReadAndWrite,
}

/// Why a dataspace, or one access's run in it, stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DataspaceError {
    #[error(transparent)]
    Contract(Box<ContractError>),
    #[error(transparent)]
    Lock(Box<LockError>),
    #[error(transparent)]
    Read(Box<ReadAllError>),
    #[error(transparent)]
    Publish(Box<PublishAllError>),
    #[error(transparent)]
    Sync(Box<SyncError>),
}

/// Why a tick or a change could not be run at all.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RunnerError {
    #[error(transparent)]
    Queue(#[from] QueueError),
    /// The Domain's list of dataspaces on autosync could not be read.
    #[error(transparent)]
    Domain(Box<crate::Error>),
}

impl From<crate::Error> for RunnerError {
    fn from(err: crate::Error) -> Self {
        Self::Domain(Box::new(err))
    }
}

/// How a job's run ends in the queue.
enum Ended {
    /// It worked, or trying again would only repeat it: it leaves the queue.
    Done,
    /// Another process held its package: it waits for the next tick.
    Again,
    /// It failed, and waits out its retry delay.
    Failed(String),
}

impl Runner {
    /// A runner on the queue at `queue_db`, usually
    /// [`crate::paths::DomainPaths::queue_db`], creating it if missing.
    pub async fn open(queue_db: &Path, settings: Settings) -> Result<Self, QueueError> {
        Ok(Self::new(Queue::open(queue_db).await?, settings))
    }

    #[must_use]
    pub fn new(queue: Queue, settings: Settings) -> Self {
        Self {
            queue,
            settings,
            ticking: Mutex::new(()),
        }
    }

    /// The queue the runner owns.
    #[must_use]
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Queues the job of each entry on autosync that publishes from the
    /// working folder holding `path`, as the contract spells it: what a
    /// folder watcher calls on a change, since a tick queues a job only where
    /// `latest` moved. A dataspace whose contract cannot be read is passed
    /// over.
    pub async fn changed(
        &self,
        storage: &(impl Storage + Sync),
        path: &Path,
    ) -> Result<Vec<JobKey>, RunnerError> {
        let mut queued = Vec::new();
        for dataspace in self.settings.domain.autosync(storage).await? {
            let local = self.settings.writer.local_registry_name();
            let Ok(contract) = Contract::read(storage, &dataspace, local).await else {
                continue;
            };
            for entry in contract.entries() {
                if entry.access != Access::Read
                    && skip(&contract, entry).is_none()
                    && path.starts_with(&entry.folder)
                {
                    let key = job_key(entry);
                    self.queue.enqueue(&key).await?;
                    queued.push(key);
                }
            }
        }
        Ok(queued)
    }

    /// One tick at `now` over every dataspace on autosync; see the module.
    /// A job that fails waits out [`crate::queue::retry_delay`] before it is
    /// tried again, counted from `now`. A kept revision is released once
    /// `latest` names it, or the lock's base is no longer the one it was
    /// built on. Only the queue failing stops the tick.
    pub async fn tick(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        now: SystemTime,
    ) -> Result<Tick, RunnerError> {
        let _one = self.ticking.lock().await;
        let mut dataspaces = Vec::new();
        for dataspace in self.settings.domain.autosync(storage).await? {
            dataspaces.push(Box::pin(self.dataspace(storage, remote, &dataspace, now)).await?);
        }
        Ok(Tick { dataspaces })
    }

    async fn dataspace(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        dataspace: &Path,
        now: SystemTime,
    ) -> Result<DataspaceTick, QueueError> {
        let mut tick = DataspaceTick {
            dataspace: dataspace.to_path_buf(),
            entries: Vec::new(),
            released: Vec::new(),
            errors: Vec::new(),
        };
        let local = self.settings.writer.local_registry_name();
        let contract = match Contract::read(storage, dataspace, local).await {
            Ok(contract) => contract,
            Err(err) => {
                tick.errors.push(DataspaceError::Contract(Box::new(err)));
                return Ok(tick);
            }
        };
        let lock = match Lock::read(storage, &contract).await {
            Ok(lock) => lock,
            Err(err) => {
                tick.errors.push(DataspaceError::Lock(Box::new(err)));
                return Ok(tick);
            }
        };
        let base = |entry: &Entry| {
            lock.as_ref()
                .and_then(|lock| lock.packages.get(&entry.key()))
                .map(|locked| locked.base.clone())
        };

        for entry in contract.entries() {
            let outcome = match skip(&contract, entry) {
                Some(skip) => EntryTick::Skipped(skip),
                None => match self.observe(remote, entry, base(entry), &mut tick).await? {
                    Ok(()) => EntryTick::Idle,
                    Err(err) => EntryTick::Unobserved(err),
                },
            };
            tick.entries.push((entry.clone(), outcome));
        }

        let jobs = self.queue.jobs().await?;
        let mut due: Vec<Entry> = Vec::new();
        for (entry, outcome) in &mut tick.entries {
            let key = job_key(entry);
            if !matches!(outcome, EntryTick::Idle) || !jobs.iter().any(|job| job.key == key) {
                continue;
            }
            match self.queue.failure(&key).await? {
                Some(failure) if failure.retry_at > now => {
                    *outcome = EntryTick::Waiting {
                        retry_at: failure.retry_at,
                    };
                }
                _ => due.push(entry.clone()),
            }
        }
        let of = |access: Access| -> Vec<Entry> {
            due.iter()
                .filter(|entry| entry.access == access)
                .cloned()
                .collect()
        };
        let publishes: Vec<Entry> = [of(Access::Write), of(Access::Replace)].concat();
        self.read(storage, remote, &of(Access::Read), now, &mut tick)
            .await?;
        self.publish(storage, remote, &publishes, &base, now, &mut tick)
            .await?;
        self.sync(storage, remote, &of(Access::Sync), now, &mut tick)
            .await?;
        Ok(tick)
    }

    /// Looks up the revision `entry` follows, releases the revision kept for
    /// its folder where that is done with, and queues its job where the
    /// revision is not `base`.
    async fn observe(
        &self,
        remote: &impl Remote,
        entry: &Entry,
        base: Option<String>,
        tick: &mut DataspaceTick,
    ) -> Result<Result<(), EntryError>, QueueError> {
        let followed = match follows(remote, entry).await {
            Ok(followed) => followed,
            Err(err) => return Ok(Err(err)),
        };
        let folder = folder_key(entry);
        if entry.access != Access::Read
            && let Some(kept) = self.queue.kept(&folder).await?
            && (followed.as_ref() == Some(&kept.hash) || base != kept.base)
        {
            self.queue.release(&folder).await?;
            tick.released.push((entry.clone(), kept));
        }
        if followed != base {
            self.queue.enqueue(&job_key(entry)).await?;
        }
        Ok(Ok(()))
    }

    async fn read(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        due: &[Entry],
        now: SystemTime,
        tick: &mut DataspaceTick,
    ) -> Result<(), QueueError> {
        if due.is_empty() {
            return Ok(());
        }
        self.start(due).await?;
        let dataspace = tick.dataspace.clone();
        let writer = &self.settings.writer;
        let read = read_some(storage, remote, &dataspace, writer, |entry| {
            due.contains(entry)
        });
        match Box::pin(read).await {
            Ok(read) => {
                for (entry, outcome) in read.entries {
                    self.end(&entry, read_ended(&outcome), now).await?;
                    set(tick, &entry, EntryTick::Read(outcome));
                }
            }
            Err(err) => {
                let err = DataspaceError::Read(Box::new(err));
                self.stop(due, err, now, tick).await?;
            }
        }
        self.end_rest(due, tick).await
    }

    async fn publish(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        due: &[Entry],
        base: &impl Fn(&Entry) -> Option<String>,
        now: SystemTime,
        tick: &mut DataspaceTick,
    ) -> Result<(), QueueError> {
        if due.is_empty() {
            return Ok(());
        }
        self.start(due).await?;
        let dataspace = tick.dataspace.clone();
        let published = publish_some(
            storage,
            remote,
            &dataspace,
            &self.settings.writer,
            &self.settings.user_rules,
            |entry| due.contains(entry),
        );
        match Box::pin(published).await {
            Ok(published) => {
                for package in published.entries {
                    let entry = package.entry;
                    // Kept as the tick keeps its own, against the base the
                    // folder was on.
                    let kept = match &package.published {
                        EntryPublish::Ran(Published::Stored { revision, .. }) => {
                            Some((revision, KeptReason::LatestMoved))
                        }
                        EntryPublish::Ran(Published::WorkflowBlocked { revision, .. }) => {
                            Some((revision, KeptReason::WorkflowBlocked))
                        }
                        _ => None,
                    };
                    if let Some((hash, reason)) = kept {
                        let revision = KeptRevision {
                            hash: hash.clone(),
                            base: base(&entry),
                            reason,
                        };
                        self.queue.keep(&folder_key(&entry), revision).await?;
                    }
                    self.end(&entry, publish_ended(&package.published), now)
                        .await?;
                    set(tick, &entry, EntryTick::Published(package.published));
                }
            }
            Err(err) => {
                let err = DataspaceError::Publish(Box::new(err));
                self.stop(due, err, now, tick).await?;
            }
        }
        self.end_rest(due, tick).await
    }

    /// Runs the tick over the dataspace's `sync` entries where any is due.
    /// It ticks every one, but only those due end a job.
    async fn sync(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        due: &[Entry],
        now: SystemTime,
        tick: &mut DataspaceTick,
    ) -> Result<(), QueueError> {
        if due.is_empty() {
            return Ok(());
        }
        self.start(due).await?;
        let settings = &self.settings;
        let dataspace = tick.dataspace.clone();
        let synced = sync(
            storage,
            remote,
            &settings.domain,
            &self.queue,
            &dataspace,
            &settings.writer,
            &settings.message,
            &settings.user_rules,
        );
        match Box::pin(synced).await {
            Ok(synced) => {
                for (entry, outcome) in synced.entries {
                    if due.contains(&entry) {
                        self.end(&entry, sync_ended(&outcome), now).await?;
                    }
                    set(tick, &entry, EntryTick::Synced(outcome));
                }
            }
            Err(err) => {
                let err = DataspaceError::Sync(Box::new(err));
                self.stop(due, err, now, tick).await?;
            }
        }
        self.end_rest(due, tick).await
    }

    async fn start(&self, due: &[Entry]) -> Result<(), QueueError> {
        for entry in due {
            self.queue.start(&job_key(entry)).await?;
        }
        Ok(())
    }

    async fn end(&self, entry: &Entry, ended: Ended, now: SystemTime) -> Result<(), QueueError> {
        let key = job_key(entry);
        match ended {
            Ended::Done => self.queue.clear_failure(&key).await?,
            Ended::Again => self.queue.enqueue(&key).await?,
            Ended::Failed(error) => {
                self.queue.fail(&key, &error, now).await?;
            }
        }
        self.queue.finish(&key).await
    }

    /// The run of `due`'s access stopped: each job counts the failure.
    async fn stop(
        &self,
        due: &[Entry],
        err: DataspaceError,
        now: SystemTime,
        tick: &mut DataspaceTick,
    ) -> Result<(), QueueError> {
        for entry in due {
            self.end(entry, Ended::Failed(err.to_string()), now).await?;
            set(tick, entry, EntryTick::Stopped);
        }
        tick.errors.push(err);
        Ok(())
    }

    /// Ends the job of each entry of `due` the run did not report, as when
    /// `data.toml` changed under it; the next tick looks again.
    async fn end_rest(&self, due: &[Entry], tick: &DataspaceTick) -> Result<(), QueueError> {
        for entry in due {
            let ran = tick
                .entries
                .iter()
                .any(|(had, outcome)| had == entry && !matches!(outcome, EntryTick::Idle));
            if !ran {
                self.queue.finish(&job_key(entry)).await?;
            }
        }
        Ok(())
    }
}

/// Why the runner leaves `entry` alone, if it does.
fn skip(contract: &Contract, entry: &Entry) -> Option<Skip> {
    if matches!(entry.registry, Registry::Local(_)) {
        return Some(Skip::LocalRegistry);
    }
    let read_too = || {
        contract
            .entries()
            .iter()
            .any(|other| other.access == Access::Read && other.key() == entry.key())
    };
    if matches!(entry.access, Access::Write | Access::Replace) && read_too() {
        return Some(Skip::ReadAndWrite);
    }
    None
}

/// The revision `entry` follows: `latest`, or the one a pinned `read` entry
/// names. `None` where `latest` names none.
async fn follows(remote: &impl Remote, entry: &Entry) -> Result<Option<String>, EntryError> {
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry);
    };
    match &entry.revision {
        RevisionPointer::Tag(Tag::Latest) => latest(remote, entry).await,
        RevisionPointer::Hash(hash) => Ok(Some(hash.clone())),
        pinned => {
            let uri = S3PackageUri {
                catalog: entry.catalog.clone(),
                bucket: bucket.clone(),
                namespace: entry.package.clone(),
                revision: pinned.clone(),
                path: None,
            };
            let resolved = resolve_manifest_uri(remote, entry.catalog.as_ref(), &uri).await?;
            Ok(Some(resolved.hash))
        }
    }
}

fn job_key(entry: &Entry) -> JobKey {
    JobKey {
        folder: folder_key(entry),
        access: entry.access,
    }
}

fn set(tick: &mut DataspaceTick, entry: &Entry, outcome: EntryTick) {
    if let Some((_, had)) = tick.entries.iter_mut().find(|(had, _)| had == entry) {
        *had = outcome;
    }
}

fn read_ended(read: &EntryRead) -> Ended {
    if read.completed() {
        return Ended::Done;
    }
    Ended::Failed(match read {
        EntryRead::Ran(ReadOutcome::Blocked {
            revision,
            conflicts,
        }) => collide(revision, conflicts.len()),
        EntryRead::Ran(ReadOutcome::Read(_)) => unread(),
        EntryRead::Unpushed => unpushed(),
        EntryRead::Failed(err) => err.to_string(),
        EntryRead::Refilled(_) => return Ended::Done,
    })
}

/// A publish stored without becoming `latest` ends its job; the revision is
/// kept in the queue.
fn publish_ended(published: &EntryPublish) -> Ended {
    Ended::Failed(match published {
        EntryPublish::Ran(Published::Behind { .. }) => {
            "`latest` is not the lock's base, so nothing was published".to_string()
        }
        EntryPublish::Ran(_) => return Ended::Done,
        EntryPublish::NotRead => unread(),
        EntryPublish::Unpushed => unpushed(),
        EntryPublish::Failed(err) => err.to_string(),
    })
}

fn sync_ended(synced: &EntrySync) -> Ended {
    Ended::Failed(match synced {
        EntrySync::Ran {
            read: Some(read), ..
        } if !read.skipped.is_empty() || !read.hash_mismatch.is_empty() => unread(),
        EntrySync::Ran { .. } | EntrySync::ReadBack(_) => return Ended::Done,
        EntrySync::Busy => return Ended::Again,
        EntrySync::Blocked {
            revision,
            conflicts,
        } => collide(revision, conflicts.len()),
        EntrySync::Diverged { .. } => {
            "`latest` is neither the lock's base nor its last commit".to_string()
        }
        EntrySync::Failed(err) => err.to_string(),
    })
}

fn collide(revision: &str, paths: usize) -> String {
    format!("local changes collide with {revision} at {paths} paths")
}

fn unread() -> String {
    "some files of the revision could not be read".to_string()
}

fn unpushed() -> String {
    "the lock names local commits not pushed".to_string()
}

#[cfg(test)]
mod tests;
