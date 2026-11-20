//! `conflict.json`, a dataspace's conflict map: for each package whose read
//! local changes blocked, each colliding key with what the revision and the
//! working folder hold there, and the decision a person or a tool returns in
//! its `resolved` field.

use std::collections::BTreeMap;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use quilt_uri::mask_user;
use serde::Deserialize;
use serde::Serialize;
use tracing::warn;

use crate::LogicalKey;
use crate::dataspace::EntryError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read::bytes_at;
use crate::dataspace::read::read_deciding;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::object_hash::ObjectHash;

/// The conflict map's file name, in the dataspace's folder.
pub const CONFLICT_FILE: &str = "conflict.json";

/// What one side holds at a colliding key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Side {
    /// Where its bytes are: an `s3://` object, or a `file://` URL in the
    /// working folder. A revision's metadata file is rendered, and has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_key: Option<String>,
    /// `None` where what is there is not a file, such as a link or a folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<ObjectHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// A key the revision being read and the working folder both changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// What the revision holds; `None` where it removed the key.
    pub incoming: Option<Side>,
    /// What the working folder holds; `None` where nothing is there.
    pub current: Option<Side>,
    /// The decision: missing leaves the key undecided, `""` deletes it, and a
    /// physical key puts that file there: a `file://` URL, an `s3://` object
    /// with its `versionId`, or the incoming object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
}

impl Row {
    /// Whether `other` describes the same incoming object and the same bytes
    /// in the working folder, however the folder's path was spelled.
    pub(crate) fn same_collision(&self, other: &Row) -> bool {
        fn bytes(side: Option<&Side>) -> Option<(Option<&ObjectHash>, Option<u64>)> {
            side.map(|side| (side.hash.as_ref(), side.size))
        }
        self.incoming == other.incoming
            && bytes(self.current.as_ref()) == bytes(other.current.as_ref())
    }
}

/// A dataspace's `conflict.json`: rows by package, then by logical key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictMap {
    pub packages: BTreeMap<PackageKey, BTreeMap<LogicalKey, Row>>,
}

impl ConflictMap {
    /// The map in the folder `dataspace`, empty where there is none.
    pub async fn read(
        storage: &(impl Storage + Sync),
        dataspace: &Path,
    ) -> Result<Self, EntryError> {
        let Some(bytes) = bytes_at(storage, &dataspace.join(CONFLICT_FILE))
            .await
            .map_err(invalid)?
        else {
            return Ok(Self::default());
        };
        let raw: BTreeMap<String, BTreeMap<LogicalKey, Row>> =
            serde_json::from_slice(&bytes).map_err(invalid)?;
        let mut packages = BTreeMap::new();
        for (text, rows) in raw {
            let key: PackageKey = text.parse().map_err(|err| {
                invalid(format!(
                    "{:?} is not a package's canonical URI: {err}",
                    mask_user(&text)
                ))
            })?;
            packages.insert(key, rows);
        }
        Ok(Self { packages })
    }

    /// Replaces the map in the folder `dataspace` whole, or removes it where
    /// no package has rows.
    pub async fn write(
        &self,
        storage: &(impl Storage + Sync),
        dataspace: &Path,
    ) -> Result<(), EntryError> {
        let path = dataspace.join(CONFLICT_FILE);
        let packages: BTreeMap<String, &BTreeMap<LogicalKey, Row>> = self
            .packages
            .iter()
            .filter(|(_, rows)| !rows.is_empty())
            .map(|(key, rows)| (key.to_string(), rows))
            .collect();
        if packages.is_empty() {
            return match storage.remove_file(&path).await {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(invalid(err)),
                _ => Ok(()),
            };
        }
        let mut text = serde_json::to_string_pretty(&packages).map_err(invalid)?;
        text.push('\n');
        // As the lock is written: a unique temporary file, not hidden.
        let temp = format!("{CONFLICT_FILE}.tmp-{}", uuid::Uuid::new_v4());
        storage
            .write_byte_stream_via(&path, temp, ByteStream::from(text.into_bytes()), None)
            .await
            .map_err(invalid)
    }
}

fn invalid(err: impl std::fmt::Display) -> EntryError {
    EntryError::ConflictMap(err.to_string())
}

/// Reads `entry` as [`read`](crate::dataspace::read::read) does, but a key
/// whose row in the `conflict.json` of the folder `dataspace` decides it, and
/// still describes what the revision and the working folder hold there, is
/// set as decided rather than colliding. A decision on a row that no longer
/// describes its key is not applied.
///
/// A read that local changes still block writes its rows to the map, keeping
/// the decisions it could apply; a read that completes removes the package's
/// rows. The map is read again before it is written, so an edit made to
/// another package's rows meanwhile is kept.
pub async fn read_resolving(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    dataspace: &Path,
    entry: &Entry,
    base: Option<&str>,
) -> Result<ReadOutcome, EntryError> {
    let key = entry.key();
    let decided = ConflictMap::read(storage, dataspace)
        .await?
        .packages
        .remove(&key)
        .unwrap_or_default();
    let (outcome, rows) = read_deciding(storage, remote, entry, base, &decided).await?;
    let blocked = matches!(outcome, ReadOutcome::Blocked { .. });
    if !blocked && (outcome.lock_base().is_none() || decided.is_empty()) {
        return Ok(outcome);
    }
    let written = async {
        let mut map = ConflictMap::read(storage, dataspace).await?;
        let mut rows = blocked.then_some(rows);
        // A decision saved while the read ran, or changed, stays.
        if let (Some(rows), Some(saved)) = (&mut rows, map.packages.get(&key)) {
            for (name, row) in rows.iter_mut() {
                if let Some(saved) = saved.get(name).filter(|saved| saved.same_collision(row)) {
                    row.resolved.clone_from(&saved.resolved);
                }
            }
        }
        // A tick that finds the same collision leaves the file as it is.
        if map.packages.get(&key) == rows.as_ref() {
            return Ok(());
        }
        match rows {
            Some(rows) => map.packages.insert(key, rows),
            None => map.packages.remove(&key),
        };
        map.write(storage, dataspace).await
    }
    .await;
    match written {
        // Nothing was applied, and without the map nothing can be decided.
        Err(err) if blocked => Err(err),
        Err(err) => {
            warn!("The read completed, and {CONFLICT_FILE} still holds its rows: {err}");
            Ok(outcome)
        }
        Ok(()) => Ok(outcome),
    }
}

#[cfg(test)]
mod tests;
