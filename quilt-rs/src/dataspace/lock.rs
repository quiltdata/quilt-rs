//! `data.lock.toml`, a dataspace's lock: each package's base and unpushed
//! local commits, under a header that names the contract, the dataspace and
//! the writer it was written for.
//!
//! A tool writes the lock after an operation completes, whole, by renaming a
//! temporary file over it. No lock file is held, so of two overlapping
//! writers the second wins.

use std::collections::BTreeMap;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::SubsecRound;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use toml::value::Datetime;

use crate::dataspace::contract::Contract;
use crate::dataspace::contract::PackageKey;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use quilt_uri::is_top_hash;

/// The lock's file name, beside the contract.
pub const LOCK_FILE: &str = "data.lock.toml";

/// The format version this crate reads and writes.
pub const LOCK_VERSION: i64 = 1;

/// Who writes a lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Writer {
    pub host: String,
    pub user: String,
    /// The name this machine's local registry keeps.
    pub local_registry: String,
}

impl Writer {
    /// This machine's host name and the user running this process.
    pub fn here(local_registry: impl Into<String>) -> Result<Self, LockError> {
        let host = hostname::get()
            .ok()
            .and_then(|host| host.into_string().ok())
            .filter(|host| !host.is_empty())
            .ok_or(LockError::Identity("host name"))?;
        let user = ["USER", "LOGNAME", "USERNAME"]
            .iter()
            .find_map(|var| std::env::var(var).ok().filter(|user| !user.is_empty()))
            .ok_or(LockError::Identity("user name"))?;
        Ok(Self {
            host,
            user,
            local_registry: local_registry.into(),
        })
    }
}

/// The lock's `[lock]` table, but for its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockHeader {
    /// The hex SHA-256 of the `data.toml` the lock was written for.
    pub data_toml_hash: String,
    /// When the dataspace was first filled; `None` until then.
    pub filled: Option<DateTime<Utc>>,
    pub modified: DateTime<Utc>,
    pub dataspace: PathBuf,
    pub writer: Writer,
}

/// A package's base, and its unpushed local commits, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockEntry {
    pub base: String,
    pub commits: Vec<String>,
}

/// A dataspace's lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    pub header: LockHeader,
    pub packages: BTreeMap<PackageKey, LockEntry>,
}

/// A part of a lock's header that is not this dataspace's, so that the lock
/// is not to be used until it is fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// `data.toml` changed since the lock was written.
    DataToml,
    Dataspace,
    LocalRegistry,
    Host,
    User,
}

impl Lock {
    /// An empty lock for `contract`'s dataspace, not yet filled.
    #[must_use]
    pub fn new(contract: &Contract, writer: &Writer) -> Self {
        Self {
            header: header(contract, writer, None),
            packages: BTreeMap::new(),
        }
    }

    /// The lock beside `contract`, or `None` where there is none.
    pub async fn read(
        storage: &(impl Storage + Sync),
        contract: &Contract,
    ) -> Result<Option<Self>, LockError> {
        let path = contract.dataspace().join(LOCK_FILE);
        match storage.read_bytes(&path).await {
            Ok(bytes) => parse(&bytes).map(Some),
            Err(err) if err.is_not_found() => Ok(None),
            Err(source) => Err(LockError::Read {
                path,
                source: Box::new(source),
            }),
        }
    }

    /// What in the header is not `contract`'s dataspace's or `writer`'s.
    #[must_use]
    pub fn mismatches(&self, contract: &Contract, writer: &Writer) -> Vec<Mismatch> {
        let header = &self.header;
        let written_by = &header.writer;
        [
            (
                header.data_toml_hash != contract.data_toml_hash(),
                Mismatch::DataToml,
            ),
            (
                header.dataspace != contract.dataspace(),
                Mismatch::Dataspace,
            ),
            (
                written_by.local_registry != writer.local_registry,
                Mismatch::LocalRegistry,
            ),
            (written_by.host != writer.host, Mismatch::Host),
            (written_by.user != writer.user, Mismatch::User),
        ]
        .into_iter()
        .filter_map(|(differs, mismatch)| differs.then_some(mismatch))
        .collect()
    }

    /// Writes the lock beside `contract`, its header made `contract`'s and
    /// `writer`'s and `filled` kept. That hides any mismatch, so a caller
    /// deals with [`Lock::mismatches`] first.
    pub async fn write(
        &mut self,
        storage: &(impl Storage + Sync),
        contract: &Contract,
        writer: &Writer,
    ) -> Result<(), LockError> {
        let header = header(contract, writer, self.header.filled);
        let text = toml::to_string(&raw(&header, &self.packages)?)?;
        let path = contract.dataspace().join(LOCK_FILE);
        // Unique, so two writers never write into one temporary file, and not
        // hidden, since nothing hidden is written in a dataspace.
        let temp = format!("{LOCK_FILE}.tmp-{}", uuid::Uuid::new_v4());
        storage
            .write_byte_stream_via(&path, temp, ByteStream::from(text.into_bytes()), None)
            .await
            .map_err(|source| LockError::Write {
                path,
                source: Box::new(source),
            })?;
        self.header = header;
        Ok(())
    }
}

fn raw(
    header: &LockHeader,
    packages: &BTreeMap<PackageKey, LockEntry>,
) -> Result<RawLock, LockError> {
    Ok(RawLock {
        lock: RawHeader {
            version: LOCK_VERSION,
            data_toml_hash: header.data_toml_hash.clone(),
            filled: header
                .filled
                .map(|time| to_toml_time(time, "filled"))
                .transpose()?,
            modified: to_toml_time(header.modified, "modified")?,
            host: header.writer.host.clone(),
            user: header.writer.user.clone(),
            dataspace: header.dataspace.clone(),
            local_registry: header.writer.local_registry.clone(),
        },
        packages: packages
            .iter()
            .map(|(key, entry)| {
                // What the reader would refuse is never written over a lock.
                let text = key.to_string();
                checked(&text, entry)?;
                let raw = if entry.commits.is_empty() {
                    RawEntry::Base(entry.base.clone())
                } else {
                    RawEntry::Commits(RawCommits {
                        base: entry.base.clone(),
                        commits: entry.commits.clone(),
                    })
                };
                Ok((text, raw))
            })
            .collect::<Result<_, LockError>>()?,
    })
}

/// The key `text` names, where it is a canonical package URI and the entry
/// holds whole top hashes.
fn checked(text: &str, entry: &LockEntry) -> Result<PackageKey, LockError> {
    let key = text.parse().map_err(|source| LockError::Key {
        key: text.to_string(),
        source,
    })?;
    if let Some(hash) = std::iter::once(&entry.base)
        .chain(&entry.commits)
        .find(|hash| !is_top_hash(hash))
    {
        return Err(LockError::Hash {
            package: text.to_string(),
            hash: hash.clone(),
        });
    }
    Ok(key)
}

/// Why a lock could not be read or written.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LockError {
    #[error("can't read {}: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: Box<crate::Error>,
    },

    #[error("can't write {}: {source}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: Box<crate::Error>,
    },

    #[error("{LOCK_FILE} is version {0}; this build reads version {LOCK_VERSION}")]
    Version(i64),

    #[error("{LOCK_FILE} is not a valid lock: {0}")]
    Syntax(#[from] toml::de::Error),

    #[error("{LOCK_FILE} names {key:?}, which is not a package's canonical URI: {source}")]
    Key {
        key: String,
        #[source]
        source: quilt_uri::UriError,
    },

    #[error("{LOCK_FILE} holds {hash:?} for {package}, which is not a whole top hash")]
    Hash { package: String, hash: String },

    #[error("{LOCK_FILE}'s `{0}` is not a UTC time")]
    Time(&'static str),

    #[error("can't write the lock as TOML: {0}")]
    Serialize(#[from] toml::ser::Error),

    #[error("can't tell this machine's {0}")]
    Identity(&'static str),
}

fn header(contract: &Contract, writer: &Writer, filled: Option<DateTime<Utc>>) -> LockHeader {
    LockHeader {
        data_toml_hash: contract.data_toml_hash().to_string(),
        filled: filled.map(|time| time.trunc_subsecs(0)),
        modified: Utc::now().trunc_subsecs(0),
        dataspace: contract.dataspace().to_path_buf(),
        writer: writer.clone(),
    }
}

fn parse(bytes: &[u8]) -> Result<Lock, LockError> {
    // The version first, so a later format is refused as one and not as a
    // lock with fields this build does not know.
    let table: toml::Table = toml::from_slice(bytes)?;
    if let Some(version) = table
        .get("lock")
        .and_then(|lock| lock.get("version"))
        .and_then(toml::Value::as_integer)
        && version != LOCK_VERSION
    {
        return Err(LockError::Version(version));
    }

    let raw: RawLock = toml::Value::Table(table).try_into()?;
    let header = raw.lock;
    let mut packages = BTreeMap::new();
    for (text, raw) in raw.packages {
        let (base, commits) = match raw {
            RawEntry::Base(base) => (base, Vec::new()),
            RawEntry::Commits(RawCommits { base, commits }) => (base, commits),
        };
        let entry = LockEntry { base, commits };
        packages.insert(checked(&text, &entry)?, entry);
    }
    Ok(Lock {
        header: LockHeader {
            data_toml_hash: header.data_toml_hash,
            filled: header
                .filled
                .map(|time| from_toml_time(time, "filled"))
                .transpose()?,
            modified: from_toml_time(header.modified, "modified")?,
            dataspace: header.dataspace,
            writer: Writer {
                host: header.host,
                user: header.user,
                local_registry: header.local_registry,
            },
        },
        packages,
    })
}

fn to_toml_time(time: DateTime<Utc>, field: &'static str) -> Result<Datetime, LockError> {
    time.format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
        .parse()
        .map_err(|_| LockError::Time(field))
}

fn from_toml_time(time: Datetime, field: &'static str) -> Result<DateTime<Utc>, LockError> {
    if !matches!(
        time.offset,
        Some(toml::value::Offset::Z | toml::value::Offset::Custom { minutes: 0 })
    ) {
        return Err(LockError::Time(field));
    }
    DateTime::parse_from_rfc3339(&time.to_string())
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| LockError::Time(field))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawLock {
    lock: RawHeader,
    #[serde(default)]
    packages: BTreeMap<String, RawEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawHeader {
    version: i64,
    data_toml_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filled: Option<Datetime>,
    modified: Datetime,
    host: String,
    user: String,
    dataspace: PathBuf,
    local_registry: String,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum RawEntry {
    Base(String),
    Commits(RawCommits),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawCommits {
    base: String,
    commits: Vec<String>,
}

#[cfg(test)]
mod tests;
