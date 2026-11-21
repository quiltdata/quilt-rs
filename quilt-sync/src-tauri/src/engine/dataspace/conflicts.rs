//! The default dataspace's `conflict.json`, as rows a person decides.
//!
//! A row is a key a read (`incoming`) or a publish (`destination`) found
//! colliding with the working folder. Deciding it writes the row's
//! `resolved` field, the one field a person writes: the folder's own file,
//! the other side's, or nothing, which deletes the key. The engine checks
//! each decision against the collision it was made for when the next read
//! or sync takes it, and drops one whose collision changed.

use quilt_rs::LogicalKey;
use quilt_rs::dataspace::conflict::Other;
use quilt_rs::dataspace::conflict::Row;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::io::remote::Remote;
use quilt_uri::Namespace;
use serde::Deserialize;
use serde::Serialize;

use super::Dataspace;
use crate::engine::error::Error;

/// What found the collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    /// A read of a newer revision, whose file is `incoming`.
    Read,
    /// A publish, whose destination holds a file it did not expect.
    Publish,
}

/// A person's decision on one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// Keep the working folder's file, or its absence.
    Mine,
    /// Take the other side's file, or its absence.
    Theirs,
    /// Neither: the key is deleted.
    Delete,
}

/// One row of `conflict.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRow {
    /// The package's canonical URI, as `conflict.json` keys it.
    pub package: String,
    pub namespace: Namespace,
    /// The logical key that collides.
    pub key: String,
    pub kind: ConflictKind,
    /// Whether the working folder holds a file there.
    pub mine: bool,
    /// Whether the other side holds one.
    pub theirs: bool,
    /// The decision already written, where one is.
    pub decision: Option<Decision>,
}

fn physical(side: Option<&quilt_rs::dataspace::conflict::Side>) -> Option<&str> {
    side.and_then(|side| side.physical_key.as_deref())
}

fn other_side(row: &Row) -> Option<&quilt_rs::dataspace::conflict::Side> {
    let (Other::Incoming(side) | Other::Destination(side)) = &row.other;
    side.as_ref()
}

/// The decision `row`'s `resolved` field names, where it names one of
/// the three.
fn decision(row: &Row) -> Option<Decision> {
    let resolved = row.resolved.as_deref()?;
    if resolved.is_empty() {
        // An empty answer keeps whichever side holds nothing.
        return Some(match (row.current.is_some(), other_side(row).is_some()) {
            (false, true) => Decision::Mine,
            (true, false) => Decision::Theirs,
            _ => Decision::Delete,
        });
    }
    if Some(resolved) == physical(row.current.as_ref()) {
        Some(Decision::Mine)
    } else if Some(resolved) == physical(other_side(row)) {
        Some(Decision::Theirs)
    } else {
        None
    }
}

/// The `resolved` text `decision` writes in `row`: the chosen side's
/// physical key, or nothing for a side that holds no file and for
/// [`Decision::Delete`].
fn resolved(row: &Row, decision: Decision) -> String {
    let side = match decision {
        Decision::Mine => row.current.as_ref(),
        Decision::Theirs => other_side(row),
        Decision::Delete => None,
    };
    physical(side).unwrap_or_default().to_string()
}

/// Every row of the default dataspace's `conflict.json`.
pub async fn rows<R: Remote + Sync>(ds: &Dataspace<R>) -> Result<Vec<ConflictRow>, Error> {
    let map = ds.conflicts().await?;
    let mut rows = Vec::new();
    for (package, keys) in &map.packages {
        for (key, row) in keys {
            rows.push(ConflictRow {
                package: package.to_string(),
                namespace: package.package.clone(),
                key: key.as_path().display().to_string(),
                kind: match row.other {
                    Other::Incoming(_) => ConflictKind::Read,
                    Other::Destination(_) => ConflictKind::Publish,
                },
                mine: row.current.is_some(),
                theirs: other_side(row).is_some(),
                decision: decision(row),
            });
        }
    }
    Ok(rows)
}

/// Writes `decision` into the row of `package` at `key`, for the next read
/// or sync to take.
pub async fn decide<R: Remote + Sync>(
    ds: &Dataspace<R>,
    package: &str,
    key: &str,
    decision: Decision,
) -> Result<(), Error> {
    let package: PackageKey = package
        .parse()
        .map_err(|err| Error::Dataspace(format!("{package}: {err}")))?;
    let key = LogicalKey::new(key).map_err(|err| Error::Dataspace(err.to_string()))?;
    let mut map = ds.conflicts().await?;
    let row = map
        .packages
        .get_mut(&package)
        .and_then(|rows| rows.get_mut(&key))
        .ok_or_else(|| {
            Error::Dataspace(format!(
                "conflict.json has no row for {} in {package}",
                key.as_path().display()
            ))
        })?;
    row.resolved = Some(resolved(row, decision));
    ds.write_conflicts(&map).await
}

#[cfg(test)]
mod tests;
