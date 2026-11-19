//! The Domain's durable queue, `queue.db` beside `state.db`: autosync's jobs,
//! the failures it waits out before trying a job again, and the revisions it
//! stored without making them `latest`.
//!
//! Nothing here can be rebuilt from elsewhere, so unlike `state.db` a file
//! this build cannot read is an error, and is never replaced.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use thiserror::Error;
use tracing::warn;

use crate::dataspace::contract::Access;
use crate::sqlite::Connection;
use crate::sqlite::ConnectionError;
use crate::state_store::FolderKey;
use crate::state_store::from_millis;
use crate::state_store::to_millis;

const SCHEMA_VERSION: i64 = 1;

/// `running_in` names the process that started a job; `NULL` while it waits.
/// `again` marks a running job queued again, which waits once its run ends.
const SCHEMA: &str = "
CREATE TABLE job (
  registry   TEXT NOT NULL,
  namespace  TEXT NOT NULL,
  folder     BLOB NOT NULL,
  access     TEXT NOT NULL,
  running_in TEXT,
  again      INTEGER NOT NULL DEFAULT 0,
  UNIQUE (folder, registry, namespace, access)
);
CREATE TABLE failure (
  registry  TEXT NOT NULL,
  namespace TEXT NOT NULL,
  folder    BLOB NOT NULL,
  access    TEXT NOT NULL,
  failures  INTEGER NOT NULL,
  error     TEXT NOT NULL,
  retry_at  INTEGER NOT NULL,
  PRIMARY KEY (folder, registry, namespace, access)
) WITHOUT ROWID;
CREATE TABLE kept (
  registry  TEXT NOT NULL,
  namespace TEXT NOT NULL,
  folder    BLOB NOT NULL,
  hash      TEXT NOT NULL,
  base      TEXT,
  reason    TEXT NOT NULL,
  PRIMARY KEY (folder, registry, namespace)
) WITHOUT ROWID;
";

const ENQUEUE: &str = "
INSERT INTO job (registry, namespace, folder, access) VALUES (?1, ?2, ?3, ?4)
ON CONFLICT DO UPDATE SET again = running_in IS NOT NULL";

/// How long a write waits for another process's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One entry's work on one working folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobKey {
    pub folder: FolderKey,
    pub access: Access,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Pending,
    /// Started by this queue and not finished.
    Running,
    /// Started by another process, which quit before finishing it or is
    /// running it still.
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub key: JobKey,
    pub state: JobState,
}

/// A job's failures since it last worked, and when to try it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub failures: u32,
    pub error: String,
    pub retry_at: SystemTime,
}

/// Why a stored revision did not become `latest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptReason {
    /// `latest` was no longer the revision the publish built from.
    LatestMoved,
    /// A workflow the registry requires refused it.
    WorkflowBlocked,
}

impl KeptReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::LatestMoved => "latest_moved",
            Self::WorkflowBlocked => "workflow_blocked",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "latest_moved" => Some(Self::LatestMoved),
            "workflow_blocked" => Some(Self::WorkflowBlocked),
            _ => None,
        }
    }
}

/// A revision stored in its registry without becoming `latest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptRevision {
    pub hash: String,
    /// What the publish built from; `None` for a package's first.
    pub base: Option<String>,
    pub reason: KeptReason,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum QueueError {
    #[error("queue: {0}")]
    Sqlite(Box<rusqlite::Error>),

    #[error("queue: {0}")]
    Io(#[from] std::io::Error),

    #[error("queue is version {0}; this build reads version {SCHEMA_VERSION}")]
    Version(i64),

    #[error("queue: a revision was kept for a reason this build doesn't know: {0:?}")]
    UnknownKeptReason(String),

    #[error("queue: the connection is closed")]
    Closed,
}

impl From<rusqlite::Error> for QueueError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(Box::new(err))
    }
}

impl From<ConnectionError> for QueueError {
    fn from(err: ConnectionError) -> Self {
        match err {
            ConnectionError::Sqlite(err) => err.into(),
            ConnectionError::Closed => Self::Closed,
        }
    }
}

/// The wait after a job's `failures`th failure in a row: 2 s, doubling to a
/// cap of 64 s.
#[must_use]
pub fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(1 << failures.clamp(1, 6))
}

pub struct Queue {
    db: Connection,
    /// Marks the jobs this queue starts, so a job another process started
    /// reads as interrupted.
    session: Arc<str>,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue").finish_non_exhaustive()
    }
}

impl Queue {
    /// Open the queue at `path`, usually
    /// [`crate::paths::DomainPaths::queue_db`], creating it if missing.
    pub async fn open(path: &Path) -> Result<Self, QueueError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Self::ready(Connection::open(path).await?).await
    }

    /// A queue that keeps nothing past the process.
    pub async fn in_memory() -> Result<Self, QueueError> {
        Self::ready(Connection::open_in_memory().await?).await
    }

    async fn ready(db: Connection) -> Result<Self, QueueError> {
        let migrated = db
            .call(|db| {
                db.busy_timeout(BUSY_TIMEOUT)?;
                migrate(db)
            })
            .await;
        if let Err(err) = migrated {
            // Closed before the caller may move the file: Windows refuses to
            // move an open one.
            let _ = db.close().await;
            return Err(err);
        }
        Ok(Self {
            db,
            session: uuid::Uuid::new_v4().to_string().into(),
        })
    }

    /// Queue `key`'s job, in its old place if it has one. A job queued while
    /// it runs goes on running, and waits again once that run finishes.
    pub async fn enqueue(&self, key: &JobKey) -> Result<(), QueueError> {
        let row = JobRow::of(key);
        self.run(move |db, _| {
            db.execute(ENQUEUE, row.params())?;
            Ok(())
        })
        .await
    }

    /// Mark `key`'s job running in this queue, queueing it if it was not.
    pub async fn start(&self, key: &JobKey) -> Result<(), QueueError> {
        let row = JobRow::of(key);
        self.run(move |db, session| {
            let (registry, namespace, folder, access) = row.params();
            db.execute(
                "INSERT INTO job (registry, namespace, folder, access, running_in)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT DO UPDATE SET
                   again = again AND running_in IS excluded.running_in,
                   running_in = excluded.running_in",
                rusqlite::params![registry, namespace, folder, access, session],
            )?;
            Ok(())
        })
        .await
    }

    /// End this queue's run of `key`'s job: it leaves the queue, unless it
    /// was queued again or failed meanwhile, when it waits again.
    pub async fn finish(&self, key: &JobKey) -> Result<(), QueueError> {
        let row = JobRow::of(key);
        self.run(move |db, session| {
            let (registry, namespace, folder, access) = row.params();
            let key = rusqlite::params![registry, namespace, folder, access, session];
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE job SET running_in = NULL, again = 0
                 WHERE registry = ?1 AND namespace = ?2 AND folder = ?3 AND access = ?4
                   AND running_in = ?5 AND again",
                key,
            )?;
            tx.execute(
                "DELETE FROM job
                 WHERE registry = ?1 AND namespace = ?2 AND folder = ?3 AND access = ?4
                   AND running_in = ?5",
                key,
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Every job, in the order it was first queued.
    pub async fn jobs(&self) -> Result<Vec<Job>, QueueError> {
        self.run(|db, session| {
            let mut query = db.prepare(
                "SELECT registry, namespace, folder, access, running_in FROM job ORDER BY rowid",
            )?;
            // A column of the wrong type reads as `None` and skips its row; a
            // failure of the database itself still ends the listing.
            let rows = query.query_map([], |row| {
                Ok((|| {
                    let key = JobRow {
                        registry: row.get(0).ok()?,
                        namespace: row.get(1).ok()?,
                        folder: row.get(2).ok()?,
                        access: row.get(3).ok()?,
                    };
                    Some((key, row.get::<_, Option<String>>(4).ok()?))
                })())
            })?;
            let mut jobs = Vec::new();
            for row in rows {
                let Some((row, running_in)) = row? else {
                    warn!("queue: skipping a job whose columns have the wrong types");
                    continue;
                };
                let state = match running_in {
                    None => JobState::Pending,
                    Some(started) if *started == *session => JobState::Running,
                    Some(_) => JobState::Interrupted,
                };
                if let Some(key) = row.into_key() {
                    jobs.push(Job { key, state });
                }
            }
            Ok(jobs)
        })
        .await
    }

    /// Count a failure of `key`'s job at `now`, and say when to try it again.
    /// The job stays queued for that try, as if queued again.
    pub async fn fail(
        &self,
        key: &JobKey,
        error: &str,
        now: SystemTime,
    ) -> Result<Failure, QueueError> {
        let row = JobRow::of(key);
        let error = error.to_owned();
        self.run(move |db, _| {
            // Locked before the read, so a writer in another process waits
            // rather than failing at the write or counting from a stale read.
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let before: u32 = tx
                .query_row(
                    "SELECT failures FROM failure
                     WHERE registry = ?1 AND namespace = ?2 AND folder = ?3 AND access = ?4",
                    row.params(),
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
            let failures = before.saturating_add(1);
            // As stored, so it equals what `failure` reads back.
            let retry_at = to_millis(now + retry_delay(failures));
            let (registry, namespace, folder, access) = row.params();
            tx.execute(
                "INSERT OR REPLACE INTO failure
                 (registry, namespace, folder, access, failures, error, retry_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    registry, namespace, folder, access, failures, error, retry_at
                ],
            )?;
            tx.execute(ENQUEUE, row.params())?;
            tx.commit()?;
            Ok(Failure {
                failures,
                error,
                retry_at: from_millis(retry_at),
            })
        })
        .await
    }

    /// `key`'s failures since its job last worked, if it has any.
    pub async fn failure(&self, key: &JobKey) -> Result<Option<Failure>, QueueError> {
        let row = JobRow::of(key);
        self.run(move |db, _| {
            Ok(db
                .query_row(
                    "SELECT failures, error, retry_at FROM failure
                     WHERE registry = ?1 AND namespace = ?2 AND folder = ?3 AND access = ?4",
                    row.params(),
                    |r| {
                        Ok(Failure {
                            failures: r.get(0)?,
                            error: r.get(1)?,
                            retry_at: from_millis(r.get(2)?),
                        })
                    },
                )
                .optional()?)
        })
        .await
    }

    /// `key`'s job worked: forget its failures.
    pub async fn clear_failure(&self, key: &JobKey) -> Result<(), QueueError> {
        let row = JobRow::of(key);
        self.run(move |db, _| {
            db.execute(
                "DELETE FROM failure
                 WHERE registry = ?1 AND namespace = ?2 AND folder = ?3 AND access = ?4",
                row.params(),
            )?;
            Ok(())
        })
        .await
    }

    /// Remember `revision` as the one `folder`'s package last stored without
    /// making it `latest`, in place of any before it.
    pub async fn keep(&self, folder: &FolderKey, revision: KeptRevision) -> Result<(), QueueError> {
        let (registry, namespace, folder) = folder.columns();
        self.run(move |db, _| {
            db.execute(
                "INSERT OR REPLACE INTO kept (registry, namespace, folder, hash, base, reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    registry,
                    namespace,
                    folder,
                    revision.hash,
                    revision.base,
                    revision.reason.as_str()
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// The revision `folder`'s package last stored without making it
    /// `latest`, unless released since.
    pub async fn kept(&self, folder: &FolderKey) -> Result<Option<KeptRevision>, QueueError> {
        let (registry, namespace, folder) = folder.columns();
        self.run(move |db, _| {
            let row = db
                .query_row(
                    "SELECT hash, base, reason FROM kept
                     WHERE registry = ?1 AND namespace = ?2 AND folder = ?3",
                    rusqlite::params![registry, namespace, folder],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            // An error, not `None`: a kept revision this build can't read is
            // still kept, and must not be taken for none.
            row.map(|(hash, base, reason)| {
                let reason = KeptReason::parse(&reason)
                    .ok_or_else(|| QueueError::UnknownKeptReason(reason))?;
                Ok(KeptRevision { hash, base, reason })
            })
            .transpose()
        })
        .await
    }

    /// Forget the revision `folder`'s package kept.
    pub async fn release(&self, folder: &FolderKey) -> Result<(), QueueError> {
        let (registry, namespace, folder) = folder.columns();
        self.run(move |db, _| {
            db.execute(
                "DELETE FROM kept WHERE registry = ?1 AND namespace = ?2 AND folder = ?3",
                rusqlite::params![registry, namespace, folder],
            )?;
            Ok(())
        })
        .await
    }

    async fn run<T, F>(&self, op: F) -> Result<T, QueueError>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Connection, &str) -> Result<T, QueueError> + Send + 'static,
    {
        let session = Arc::clone(&self.session);
        self.db.call(move |db| op(db, &session)).await
    }
}

/// Creating the tables takes the write lock, and reads the version again
/// under it, so two processes opening a new file create them once; a file
/// already made is opened without it.
fn migrate(db: &mut rusqlite::Connection) -> Result<(), QueueError> {
    let version = |db: &rusqlite::Connection| -> rusqlite::Result<i64> {
        db.query_row("PRAGMA user_version", [], |row| row.get(0))
    };
    match version(db)? {
        SCHEMA_VERSION => return Ok(()),
        0 => {}
        other => return Err(QueueError::Version(other)),
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match version(&tx)? {
        SCHEMA_VERSION => Ok(()),
        0 => {
            tx.execute_batch(SCHEMA)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
            Ok(())
        }
        other => Err(QueueError::Version(other)),
    }
}

/// A job's key as SQLite holds it.
struct JobRow {
    registry: String,
    namespace: String,
    folder: Vec<u8>,
    access: String,
}

impl JobRow {
    fn of(key: &JobKey) -> Self {
        let (registry, namespace, folder) = key.folder.columns();
        Self {
            registry,
            namespace,
            folder,
            access: key.access.to_string(),
        }
    }

    fn params(&self) -> (&str, &str, &[u8], &str) {
        (&self.registry, &self.namespace, &self.folder, &self.access)
    }

    /// `None`, logged, for a row this build can't read: it is skipped rather
    /// than holding up every other job.
    fn into_key(self) -> Option<JobKey> {
        let Ok(access) = self.access.parse() else {
            warn!("queue: skipping a job with access {:?}", self.access);
            return None;
        };
        let Some(folder) = FolderKey::from_columns(self.registry, &self.namespace, &self.folder)
        else {
            warn!("queue: skipping a job of {:?}", self.namespace);
            return None;
        };
        Some(JobKey { folder, access })
    }
}

#[cfg(test)]
mod tests;
