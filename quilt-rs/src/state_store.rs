//! Per-package state the desktop app keeps beside the lineage: `.quilt/state.db`.
//!
//! Optional and additional. Nothing in it is needed to read or write a package,
//! so a missing or damaged file gives the same results, only later. No method
//! returns an error: a failure is logged, the file is created again, and a read
//! that still fails answers "nothing stored".

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use multihash::Multihash;
use quilt_uri::Namespace;
use tokio::sync::Mutex;
use tokio_rusqlite::Connection;
use tokio_rusqlite::rusqlite;
use tokio_rusqlite::rusqlite::OptionalExtension;
use tracing::info;
use tracing::warn;

use crate::file_stamp::FileId;
use crate::file_stamp::FileStamp;
use crate::object_hash::ObjectHash;
use crate::stat_cache::CachedHash;
use crate::stat_cache::FULL_HASH_EVERY;
use crate::stat_cache::StatCache;

const SCHEMA_VERSION: i64 = 2;

/// Each step takes the schema from the version before it to its own.
const MIGRATIONS: [&str; 2] = [
    "
CREATE TABLE package_activity (
  namespace      TEXT PRIMARY KEY,
  last_change_at INTEGER NOT NULL,
  source         TEXT NOT NULL
);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
",
    // Times are unix nanoseconds; `hash` is the multihash bytes.
    "
CREATE TABLE file_stat (
  namespace TEXT NOT NULL,
  path      TEXT NOT NULL,
  algorithm INTEGER NOT NULL,
  size      INTEGER NOT NULL,
  mtime     INTEGER NOT NULL,
  ctime     INTEGER NOT NULL,
  volume    INTEGER NOT NULL,
  inode     BLOB NOT NULL,
  hash      BLOB NOT NULL,
  saved_at  INTEGER NOT NULL,
  PRIMARY KEY (namespace, path, algorithm)
) WITHOUT ROWID;
CREATE TABLE stat_walk (
  namespace    TEXT PRIMARY KEY,
  full_hash_at INTEGER NOT NULL
);
",
];

/// What moved a package's last-change time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeSource {
    /// The folder watcher heard a change in the package.
    Edit,
    /// Publishing was turned on while the package had changes.
    PublishingOn,
    /// A walk found changes the app had no time for.
    FirstSeen,
    /// The package was committed in the app; the commit waits to be pushed.
    Commit,
}

impl ChangeSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::PublishingOn => "publishing_on",
            Self::FirstSeen => "first_seen",
            Self::Commit => "commit",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "edit" => Some(Self::Edit),
            "publishing_on" => Some(Self::PublishingOn),
            "first_seen" => Some(Self::FirstSeen),
            "commit" => Some(Self::Commit),
            _ => None,
        }
    }
}

/// A package's last-change time and what set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageActivity {
    pub last_change_at: SystemTime,
    pub source: ChangeSource,
}

#[derive(Debug)]
enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// The connection's worker thread is gone.
    Closed,
    /// Written by a newer app. The store holds nothing that can't be rebuilt,
    /// so it is replaced rather than read.
    NewerSchema(i64),
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "{err}"),
            Self::Io(err) => write!(f, "{err}"),
            Self::Closed => write!(f, "connection closed"),
            Self::NewerSchema(version) => {
                write!(f, "schema version {version} is newer than this app's")
            }
        }
    }
}

impl StoreError {
    /// Whether the file itself is bad, so starting a new one is the fix.
    fn is_damage(&self) -> bool {
        match self {
            Self::NewerSchema(_) => true,
            Self::Sqlite(rusqlite::Error::SqliteFailure(err, _)) => matches!(
                err.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ),
            Self::Sqlite(_) | Self::Io(_) | Self::Closed => false,
        }
    }
}

struct Open {
    connection: Connection,
    /// `false` once the file could not be created and the tables live in
    /// memory for the rest of the session.
    on_disk: bool,
}

/// The package state store. Opens its file on first use.
pub struct PackageStateStore {
    /// `None` keeps the tables in memory only.
    path: Option<PathBuf>,
    open: Mutex<Option<Open>>,
}

impl std::fmt::Debug for PackageStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackageStateStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PackageStateStore {
    /// A store kept at `path`, usually [`crate::paths::DomainPaths::state_db`].
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            open: Mutex::new(None),
        }
    }

    /// A store that keeps nothing past the process.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            path: None,
            open: Mutex::new(None),
        }
    }

    /// The package's last-change time, if one is stored.
    pub async fn last_change(&self, namespace: &Namespace) -> Option<PackageActivity> {
        let key = namespace.to_string();
        self.run(move |db| {
            Ok(db
                .query_row(
                    "SELECT last_change_at, source FROM package_activity WHERE namespace = ?1",
                    [&key],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?)
        })
        .await
        .flatten()
        .and_then(|(at, source)| {
            Some(PackageActivity {
                last_change_at: from_millis(at),
                source: ChangeSource::parse(&source)?,
            })
        })
    }

    /// Set the package's last-change time, replacing any stored one.
    pub async fn record_change(&self, namespace: &Namespace, at: SystemTime, source: ChangeSource) {
        let key = namespace.to_string();
        let at = to_millis(at);
        self.run(move |db| {
            db.execute(
                "INSERT INTO package_activity (namespace, last_change_at, source) VALUES (?1, ?2, ?3)
                 ON CONFLICT(namespace) DO UPDATE SET last_change_at = ?2, source = ?3",
                rusqlite::params![key, at, source.as_str()],
            )?;
            Ok(())
        })
        .await;
    }

    /// Store `at` as a first sighting unless the package already has a time.
    /// Returns whether it stored one.
    pub async fn record_first_seen(&self, namespace: &Namespace, at: SystemTime) -> bool {
        let key = namespace.to_string();
        let at = to_millis(at);
        self.run(move |db| {
            Ok(db.execute(
                "INSERT OR IGNORE INTO package_activity (namespace, last_change_at, source)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![key, at, ChangeSource::FirstSeen.as_str()],
            )? == 1)
        })
        .await
        .unwrap_or(false)
    }

    /// Delete the package's last-change time.
    pub async fn forget(&self, namespace: &Namespace) {
        let key = namespace.to_string();
        self.run(move |db| {
            db.execute("DELETE FROM package_activity WHERE namespace = ?1", [&key])?;
            Ok(())
        })
        .await;
    }

    /// Every package with a stored last-change time.
    pub async fn namespaces(&self) -> Vec<Namespace> {
        self.run(|db| {
            let mut query = db.prepare("SELECT namespace FROM package_activity")?;
            let rows = query
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|key| Namespace::try_from(key.as_str()).ok())
        .collect()
    }

    /// The package's [`StatCache`] for one walk. A walk a day hashes every
    /// file anyway; so does one the store can't answer.
    pub async fn stat_cache(&self, namespace: &Namespace) -> StatCache {
        let key = namespace.to_string();
        let loaded = self
            .run(move |db| {
                let full_hash_at = db
                    .query_row(
                        "SELECT full_hash_at FROM stat_walk WHERE namespace = ?1",
                        [&key],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                let mut query = db.prepare(
                    "SELECT path, algorithm, size, mtime, ctime, volume, inode, hash, saved_at
                     FROM file_stat WHERE namespace = ?1",
                )?;
                let rows = query
                    .query_map([&key], |row| {
                        Ok(StatRow {
                            path: row.get(0)?,
                            algorithm: row.get(1)?,
                            size: row.get(2)?,
                            mtime: row.get(3)?,
                            ctime: row.get(4)?,
                            volume: row.get(5)?,
                            inode: row.get(6)?,
                            hash: row.get(7)?,
                            saved_at: row.get(8)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((full_hash_at, rows))
            })
            .await;
        let Some((full_hash_at, rows)) = loaded else {
            return StatCache::new(HashMap::new(), true);
        };
        let full = full_hash_at.is_none_or(|at| {
            SystemTime::now()
                .duration_since(from_nanos(at))
                .is_ok_and(|since| since >= FULL_HASH_EVERY)
        });
        let saved = rows.into_iter().filter_map(StatRow::into_entry).collect();
        StatCache::new(saved, full)
    }

    /// Write what a walk with `cache` changed: new and changed entries, and
    /// the deletion of files it did not find.
    pub async fn save_stat_cache(&self, namespace: &Namespace, cache: &StatCache) {
        let gone = cache.gone();
        if cache.updates().is_empty() && gone.is_empty() && !cache.is_full() {
            return;
        }
        let key = namespace.to_string();
        let updates: Arc<Vec<StatRow>> = Arc::new(
            cache
                .updates()
                .iter()
                .filter_map(|(path, entry)| StatRow::from_entry(path, entry))
                .collect(),
        );
        let gone: Arc<Vec<String>> = Arc::new(
            gone.iter()
                .filter_map(|path| path.to_str().map(str::to_owned))
                .collect(),
        );
        let full_hash_at = cache.is_full().then(|| to_nanos(SystemTime::now()));
        self.run(move |db| {
            let tx = db.unchecked_transaction()?;
            {
                let mut delete =
                    tx.prepare_cached("DELETE FROM file_stat WHERE namespace = ?1 AND path = ?2")?;
                for path in gone.iter() {
                    delete.execute(rusqlite::params![key, path])?;
                }
                let mut upsert = tx.prepare_cached(
                    "INSERT OR REPLACE INTO file_stat
                     (namespace, path, algorithm, size, mtime, ctime, volume, inode, hash, saved_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )?;
                for row in updates.iter() {
                    upsert.execute(rusqlite::params![
                        key,
                        row.path,
                        row.algorithm,
                        row.size,
                        row.mtime,
                        row.ctime,
                        row.volume,
                        row.inode,
                        row.hash,
                        row.saved_at,
                    ])?;
                }
                if let Some(at) = full_hash_at {
                    tx.execute(
                        "INSERT OR REPLACE INTO stat_walk (namespace, full_hash_at) VALUES (?1, ?2)",
                        rusqlite::params![key, at],
                    )?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await;
    }

    /// Run `op`, opening the store first if needed. A damaged file is replaced
    /// and `op` tried once more; any other failure answers `None`.
    async fn run<T, F>(&self, op: F) -> Option<T>
    where
        T: Send + 'static,
        F: Fn(&rusqlite::Connection) -> Result<T, StoreError> + Clone + Send + 'static,
    {
        let mut open = self.open.lock().await;
        let mut discarded = false;
        for _ in 0..3 {
            // A file deleted under an open connection would keep answering from
            // the unlinked copy, so its absence means "start again".
            let gone = open
                .as_ref()
                .is_some_and(|o| o.on_disk && self.path.as_deref().is_some_and(|p| !p.exists()));
            if gone {
                info!("state store: {:?} is gone, creating it again", self.path);
                *open = None;
            }
            if open.is_none() {
                *open = self.open_connection().await;
            }
            let current = open.as_ref()?;
            let (connection, on_disk) = (current.connection.clone(), current.on_disk);
            let op = op.clone();
            match connection.call(move |db| op(db)).await {
                Ok(value) => return Some(value),
                Err(tokio_rusqlite::Error::Error(err))
                    if on_disk && !discarded && err.is_damage() =>
                {
                    warn!("state store: {err}; starting a new one");
                    // Closed first: Windows refuses to delete an open file.
                    if let Some(old) = open.take() {
                        let _ = old.connection.close().await;
                    }
                    self.discard_file();
                    discarded = true;
                }
                // A file that answers but can't keep a write would leave its old
                // times in charge. Memory keeps the new ones; the file is left
                // as it is.
                Err(err) if on_disk => {
                    warn!("state store: {err}; keeping state in memory");
                    *open = open_memory_or_log().await;
                }
                Err(err) => {
                    warn!("state store: {err}");
                    return None;
                }
            }
        }
        None
    }

    async fn open_connection(&self) -> Option<Open> {
        if let Some(path) = &self.path {
            match open_file(path).await {
                Ok(connection) => {
                    return Some(Open {
                        connection,
                        on_disk: true,
                    });
                }
                // Only a damaged file is deleted. Any other failure, such as a
                // lock or a read error, may pass, and the file may hold good times.
                Err(err) if err.is_damage() => {
                    warn!("state store: {}: {err}; starting a new one", path.display());
                    self.discard_file();
                    match open_file(path).await {
                        Ok(connection) => {
                            return Some(Open {
                                connection,
                                on_disk: true,
                            });
                        }
                        Err(err) => warn!(
                            "state store: cannot create {}: {err}; keeping state in memory",
                            path.display()
                        ),
                    }
                }
                Err(err) => warn!(
                    "state store: cannot open {}: {err}; keeping state in memory",
                    path.display()
                ),
            }
        }
        open_memory_or_log().await
    }

    fn discard_file(&self) {
        let Some(path) = &self.path else {
            return;
        };
        for file in [
            path.clone(),
            sidecar(path, "-journal"),
            sidecar(path, "-wal"),
        ] {
            if let Err(err) = std::fs::remove_file(&file)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                warn!("state store: cannot remove {}: {err}", file.display());
            }
        }
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

async fn open_file(path: &Path) -> Result<Connection, StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
    }
    let connection = Connection::open(path).await?;
    if let Err(err) = migrate(&connection).await {
        // Closed before the caller may delete the file: Windows refuses to
        // delete an open one.
        let _ = connection.close().await;
        return Err(err);
    }
    Ok(connection)
}

async fn open_memory_or_log() -> Option<Open> {
    match open_memory().await {
        Ok(connection) => Some(Open {
            connection,
            on_disk: false,
        }),
        Err(err) => {
            warn!("state store: cannot open in memory: {err}");
            None
        }
    }
}

async fn open_memory() -> Result<Connection, StoreError> {
    let connection = Connection::open_in_memory().await?;
    migrate(&connection).await?;
    Ok(connection)
}

async fn migrate(connection: &Connection) -> Result<(), StoreError> {
    connection
        .call(|db| {
            let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            if version > SCHEMA_VERSION {
                return Err(StoreError::NewerSchema(version));
            }
            if version == SCHEMA_VERSION {
                return Ok(());
            }
            let tx = db.transaction()?;
            for step in MIGRATIONS
                .iter()
                .skip(usize::try_from(version).unwrap_or(0))
            {
                tx.execute_batch(step)?;
            }
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|err| match err {
            tokio_rusqlite::Error::Error(err) => err,
            _ => StoreError::Closed,
        })
}

/// A `file_stat` row as SQLite holds it.
struct StatRow {
    path: String,
    algorithm: i64,
    size: i64,
    mtime: i64,
    ctime: i64,
    volume: i64,
    inode: Vec<u8>,
    hash: Vec<u8>,
    saved_at: i64,
}

impl StatRow {
    fn from_entry(path: &Path, entry: &CachedHash) -> Option<Self> {
        let id = entry.stamp.id?;
        Some(Self {
            path: path.to_str()?.to_owned(),
            algorithm: i64::try_from(entry.hash.algorithm()).ok()?,
            size: i64::try_from(entry.stamp.size).ok()?,
            mtime: to_nanos(entry.stamp.modified),
            ctime: to_nanos(entry.stamp.changed?),
            volume: i64::from_ne_bytes(id.volume.to_ne_bytes()),
            inode: id.file.to_le_bytes().to_vec(),
            hash: entry.hash.multihash().to_bytes(),
            saved_at: to_nanos(entry.saved_at),
        })
    }

    /// `None` for a row this build can't read, which then counts as not cached.
    fn into_entry(self) -> Option<((PathBuf, u64), CachedHash)> {
        let hash = Multihash::<256>::from_bytes(&self.hash).ok()?;
        let entry = CachedHash {
            stamp: FileStamp {
                size: u64::try_from(self.size).ok()?,
                modified: from_nanos(self.mtime),
                changed: Some(from_nanos(self.ctime)),
                id: Some(FileId {
                    volume: u64::from_ne_bytes(self.volume.to_ne_bytes()),
                    file: u128::from_le_bytes(self.inode.try_into().ok()?),
                }),
            },
            hash: ObjectHash::try_from(hash).ok()?,
            saved_at: from_nanos(self.saved_at),
        };
        let algorithm = u64::try_from(self.algorithm).ok()?;
        (entry.hash.algorithm() == algorithm)
            .then(|| ((PathBuf::from(self.path), algorithm), entry))
    }
}

fn to_nanos(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

fn from_nanos(ns: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(ns.unsigned_abs());
    if ns >= 0 {
        UNIX_EPOCH + magnitude
    } else {
        UNIX_EPOCH - magnitude
    }
}

fn to_millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn from_millis(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    type Res = Result<(), Box<dyn std::error::Error>>;

    fn ns() -> Namespace {
        ("acme", "demo").into()
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test(tokio::test)]
    async fn a_missing_file_is_created_on_first_use() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join(".quilt").join("state.db");
        let store = PackageStateStore::new(path.clone());
        assert_eq!(store.last_change(&ns()).await, None);
        assert!(path.exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_change_survives_reopening() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("state.db");
        PackageStateStore::new(path.clone())
            .record_change(&ns(), at(100), ChangeSource::Edit)
            .await;
        let store = PackageStateStore::new(path);
        assert_eq!(
            store.last_change(&ns()).await,
            Some(PackageActivity {
                last_change_at: at(100),
                source: ChangeSource::Edit,
            })
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn record_change_replaces_and_first_seen_does_not() -> Res {
        let store = PackageStateStore::in_memory();
        assert!(store.record_first_seen(&ns(), at(10)).await);
        store
            .record_change(&ns(), at(20), ChangeSource::PublishingOn)
            .await;
        assert!(!store.record_first_seen(&ns(), at(30)).await);
        assert_eq!(
            store.last_change(&ns()).await,
            Some(PackageActivity {
                last_change_at: at(20),
                source: ChangeSource::PublishingOn,
            })
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn forget_deletes_one_package() -> Res {
        let store = PackageStateStore::in_memory();
        let other: Namespace = ("acme", "other").into();
        store.record_change(&ns(), at(1), ChangeSource::Edit).await;
        store.record_change(&other, at(2), ChangeSource::Edit).await;
        store.forget(&ns()).await;
        assert_eq!(store.last_change(&ns()).await, None);
        assert_eq!(store.namespaces().await, vec![other]);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_damaged_file_is_replaced() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("state.db");
        std::fs::write(
            &path,
            b"this is not a database, just some bytes long enough",
        )?;
        let store = PackageStateStore::new(path.clone());
        assert_eq!(store.last_change(&ns()).await, None);
        store.record_change(&ns(), at(3), ChangeSource::Edit).await;
        assert_eq!(
            PackageStateStore::new(path)
                .last_change(&ns())
                .await
                .map(|a| a.last_change_at),
            Some(at(3))
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_file_from_a_newer_app_is_replaced() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("state.db");
        {
            let db = rusqlite::Connection::open(&path)?;
            db.pragma_update(None, "user_version", SCHEMA_VERSION + 1)?;
        }
        let store = PackageStateStore::new(path);
        store.record_change(&ns(), at(4), ChangeSource::Edit).await;
        assert_eq!(
            store.last_change(&ns()).await.map(|a| a.last_change_at),
            Some(at(4))
        );
        Ok(())
    }

    /// Windows refuses to delete a file that is open.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_file_deleted_while_open_starts_empty() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("state.db");
        let store = PackageStateStore::new(path.clone());
        store.record_change(&ns(), at(1), ChangeSource::Edit).await;
        std::fs::remove_file(&path)?;
        assert_eq!(store.last_change(&ns()).await, None);
        assert!(path.exists());
        store.record_change(&ns(), at(2), ChangeSource::Edit).await;
        assert_eq!(
            store.last_change(&ns()).await.map(|a| a.last_change_at),
            Some(at(2))
        );
        Ok(())
    }

    /// A read-only file opens and answers reads but refuses writes.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_file_that_refuses_writes_keeps_new_times_in_memory() -> Res {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("state.db");
        PackageStateStore::new(path.clone())
            .record_change(&ns(), at(1), ChangeSource::Edit)
            .await;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444))?;

        let store = PackageStateStore::new(path.clone());
        store.record_change(&ns(), at(2), ChangeSource::Edit).await;
        assert_eq!(
            store.last_change(&ns()).await.map(|a| a.last_change_at),
            Some(at(2)),
            "the new time, not the file's old one"
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        assert_eq!(
            PackageStateStore::new(path)
                .last_change(&ns())
                .await
                .map(|a| a.last_change_at),
            Some(at(1)),
            "the file is kept, not deleted"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_version_1_file_keeps_its_times_and_gains_the_stat_cache() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("state.db");
        {
            let db = rusqlite::Connection::open(&path)?;
            db.execute_batch(MIGRATIONS[0])?;
            db.pragma_update(None, "user_version", 1)?;
            db.execute(
                "INSERT INTO package_activity VALUES ('acme/demo', 5000, 'edit')",
                [],
            )?;
        }
        let store = PackageStateStore::new(path);
        assert_eq!(
            store.last_change(&ns()).await.map(|a| a.last_change_at),
            Some(at(5))
        );
        assert!(store.stat_cache(&ns()).await.is_full(), "never walked");
        Ok(())
    }

    fn cached() -> CachedHash {
        CachedHash {
            stamp: FileStamp {
                size: 3,
                modified: at(100),
                changed: Some(at(100)),
                id: Some(FileId {
                    volume: u64::MAX,
                    file: u128::MAX - 1,
                }),
            },
            hash: crate::object_hash::Crc64Hash::default().into(),
            saved_at: at(200),
        }
    }

    #[test(tokio::test)]
    async fn a_walk_a_day_hashes_everything() -> Res {
        let store = PackageStateStore::in_memory();
        let first = store.stat_cache(&ns()).await;
        assert!(first.is_full(), "no full walk yet");
        store.save_stat_cache(&ns(), &first).await;
        assert!(!store.stat_cache(&ns()).await.is_full());

        let a_day_ago = to_nanos(SystemTime::now() - FULL_HASH_EVERY);
        let key = ns().to_string();
        store
            .run(move |db| {
                db.execute(
                    "UPDATE stat_walk SET full_hash_at = ?2 WHERE namespace = ?1",
                    rusqlite::params![key, a_day_ago],
                )?;
                Ok(())
            })
            .await;
        assert!(store.stat_cache(&ns()).await.is_full());
        Ok(())
    }

    #[test(tokio::test)]
    async fn stat_entries_round_trip_and_unseen_ones_are_deleted() -> Res {
        let store = PackageStateStore::in_memory();
        let mut cache = store.stat_cache(&ns()).await;
        cache.record(std::path::Path::new("a"), cached());
        cache.record(std::path::Path::new("b"), cached());
        store.save_stat_cache(&ns(), &cache).await;

        let mut cache = store.stat_cache(&ns()).await;
        let entry = cached();
        assert_eq!(
            cache.lookup(
                std::path::Path::new("a"),
                &entry.stamp,
                entry.hash.algorithm()
            ),
            Some(&entry.hash)
        );
        cache.see(std::path::Path::new("a"));
        store.save_stat_cache(&ns(), &cache).await;

        let cache = store.stat_cache(&ns()).await;
        assert_eq!(
            cache.lookup(
                std::path::Path::new("b"),
                &entry.stamp,
                entry.hash.algorithm()
            ),
            None
        );
        assert!(
            cache
                .lookup(
                    std::path::Path::new("a"),
                    &entry.stamp,
                    entry.hash.algorithm()
                )
                .is_some()
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_uncreatable_file_keeps_state_in_memory() -> Res {
        let dir = tempfile::TempDir::new()?;
        // A file where the parent folder should be.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"")?;
        let store = PackageStateStore::new(blocker.join("state.db"));
        store.record_change(&ns(), at(9), ChangeSource::Edit).await;
        assert_eq!(
            store.last_change(&ns()).await.map(|a| a.last_change_at),
            Some(at(9))
        );
        Ok(())
    }
}
