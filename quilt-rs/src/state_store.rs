//! Per-package state the desktop app keeps beside the lineage: `.quilt/state.db`.
//!
//! Optional and additional. Nothing in it is needed to read or write a package,
//! so a missing or damaged file gives the same results, only later. No method
//! returns an error: a failure is logged, the file is created again, and a read
//! that still fails answers "nothing stored".

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use quilt_uri::Namespace;
use tokio::sync::Mutex;
use tokio_rusqlite::Connection;
use tokio_rusqlite::rusqlite;
use tokio_rusqlite::rusqlite::OptionalExtension;
use tracing::info;
use tracing::warn;

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE package_activity (
  namespace      TEXT PRIMARY KEY,
  last_change_at INTEGER NOT NULL,
  source         TEXT NOT NULL
);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

const LAST_ALIVE_AT: &str = "last_alive_at";

/// What moved a package's last-change time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeSource {
    /// The folder watcher heard a change in the package.
    Edit,
    /// Publishing was turned on while the package had changes.
    PublishingOn,
    /// A walk found changes the app had no time for.
    FirstSeen,
}

impl ChangeSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::PublishingOn => "publishing_on",
            Self::FirstSeen => "first_seen",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "edit" => Some(Self::Edit),
            "publishing_on" => Some(Self::PublishingOn),
            "first_seen" => Some(Self::FirstSeen),
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

    /// The last time the app recorded that it was running.
    pub async fn last_alive_at(&self) -> Option<SystemTime> {
        self.run(|db| {
            Ok(db
                .query_row(
                    "SELECT value FROM meta WHERE key = ?1",
                    [LAST_ALIVE_AT],
                    |row| row.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .map(from_millis)
    }

    pub async fn set_last_alive_at(&self, at: SystemTime) {
        let value = to_millis(at).to_string();
        self.run(move |db| {
            db.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                rusqlite::params![LAST_ALIVE_AT, value],
            )?;
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
        for attempt in 0..2 {
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
            let connection = open.as_ref()?.connection.clone();
            let op = op.clone();
            match connection.call(move |db| op(db)).await {
                Ok(value) => return Some(value),
                Err(tokio_rusqlite::Error::Error(err)) if attempt == 0 && err.is_damage() => {
                    warn!("state store: {err}; starting a new one");
                    *open = None;
                    self.discard_file();
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
                Err(err) => warn!(
                    "state store: cannot open {}: {err}; starting a new one",
                    path.display()
                ),
            }
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
    migrate(&connection).await?;
    Ok(connection)
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
            match version {
                0 => {
                    let tx = db.transaction()?;
                    tx.execute_batch(SCHEMA)?;
                    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                    tx.commit()?;
                    Ok(())
                }
                SCHEMA_VERSION => Ok(()),
                newer => Err(StoreError::NewerSchema(newer)),
            }
        })
        .await
        .map_err(|err| match err {
            tokio_rusqlite::Error::Error(err) => err,
            _ => StoreError::Closed,
        })
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
    async fn last_alive_at_round_trips() -> Res {
        let store = PackageStateStore::in_memory();
        assert_eq!(store.last_alive_at().await, None);
        store.set_last_alive_at(at(5)).await;
        store.set_last_alive_at(at(7)).await;
        assert_eq!(store.last_alive_at().await, Some(at(7)));
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
