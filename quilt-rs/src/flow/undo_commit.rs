//! Discard a package's newest local commit and restore the revision before it.
//!
//! [`CommitState::prev_hashes`](crate::lineage::CommitState) is the only
//! on-disk record of a revision's parent, and [`push`](super::push) consumes it
//! while [`reset_to_latest`](super::reset_to_latest) clears it. That chain is
//! what bounds how far undo reaches.
//!
//! Contract in `docs/architecture.md` — Operation Contracts, Undo Commit.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use tracing::{debug, info};

use crate::Error;
use crate::Res;
use crate::checksum::refresh_hash;
use crate::error::PackageOpError;
use crate::io::storage::Storage;
use crate::lineage::{CommitState, PackageLineage, PathState};
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

use super::pull_outcome::{RemoteChange, remote_delta};

/// Not a transaction and not resumable: a part-way failure leaves a mix of both
/// revisions, which [`refuse_unless_clean`] then declines to write over.
pub async fn undo_commit(
    mut lineage: PackageLineage,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    package_home: PathBuf,
    namespace: Namespace,
) -> Res<(PackageLineage, CommitState)> {
    let current = lineage.commit.clone().ok_or_else(|| {
        Error::PackageOp(PackageOpError::Undo(
            "package has no local commit to undo".to_string(),
        ))
    })?;
    let previous_hash = current.prev_hashes.first().cloned().ok_or_else(|| {
        Error::PackageOp(PackageOpError::Undo(
            "this is the package's first revision, so there is nothing to undo".to_string(),
        ))
    })?;

    let current_manifest = Manifest::from_path(
        storage,
        &paths.installed_manifest(&namespace, &current.hash),
    )
    .await?;
    let previous_manifest_path = paths.installed_manifest(&namespace, &previous_hash);
    let previous_manifest = Manifest::from_path(storage, &previous_manifest_path).await?;

    let delta = remote_delta(&current_manifest, &previous_manifest);

    refuse_unless_clean(&lineage, storage, &package_home, &current_manifest, &delta).await?;

    info!(
        "⏳ Undoing commit {} of {}, restoring {}",
        current.hash, namespace, previous_hash
    );

    // Both arms mean: the restored revision has content here, the tree does not.
    let to_write: Vec<(&PathBuf, &crate::object_hash::ObjectHash)> = delta
        .iter()
        .filter_map(|(key, change)| match change {
            RemoteChange::Modified(hash) | RemoteChange::Added(hash) => Some((key, hash)),
            RemoteChange::Removed => None,
        })
        .collect();

    // Removals run last so a failure cannot hole the tree — except where a
    // path's type changed: a file at `a` blocks creating `a/b`, and a directory
    // at `a` blocks writing the file `a`.
    let (blocking, trailing): (Vec<&PathBuf>, Vec<&PathBuf>) = delta
        .iter()
        .filter(|(_, change)| matches!(change, RemoteChange::Removed))
        .map(|(key, _)| key)
        .partition(|removed| to_write.iter().any(|(write, _)| nests(removed, write)));

    let mut restored_paths = lineage.paths.clone();
    for logical_key in blocking {
        remove_working_file(storage, &package_home, logical_key).await?;
        restored_paths.remove(logical_key);
    }

    for (logical_key, hash) in &to_write {
        let working_path = package_home.join(logical_key);
        if let Some(parent) = working_path.parent() {
            storage.create_dir_all(parent).await?;
        }
        debug!("⏳ Restoring {}", logical_key.display());
        storage
            .copy(&paths.object(hash.digest()), &working_path)
            .await?;
        restored_paths.insert(
            (*logical_key).clone(),
            PathState {
                timestamp: storage.modified_timestamp(&working_path).await?,
                hash: (*hash).clone().into(),
            },
        );
    }

    for logical_key in trailing {
        remove_working_file(storage, &package_home, logical_key).await?;
        restored_paths.remove(logical_key);
    }

    let commit = CommitState {
        timestamp: storage.modified_timestamp(&previous_manifest_path).await?,
        hash: previous_hash,
        prev_hashes: current.prev_hashes.into_iter().skip(1).collect(),
    };
    lineage.paths = restored_paths;
    lineage.commit = Some(commit.clone());

    info!("✔️ Undid commit of {}, now at {}", namespace, commit.hash);
    Ok((lineage, commit))
}

/// Whether two keys contend for the same place on disk (`a` vs `a/b`).
fn nests(one: &Path, other: &Path) -> bool {
    one.starts_with(other) || other.starts_with(one)
}

/// Remove a working file, pruning directories it empties (below the package
/// home). A file cannot be written where an emptied directory stands.
async fn remove_working_file(
    storage: &(impl Storage + Sync),
    package_home: &Path,
    logical_key: &Path,
) -> Res {
    let working_path = package_home.join(logical_key);
    debug!("⏳ Removing {}", logical_key.display());
    if let Err(err) = storage.remove_file(&working_path).await
        && err.kind() != ErrorKind::NotFound
    {
        return Err(Error::Io(err));
    }

    let mut dir = working_path.parent().map(Path::to_path_buf);
    while let Some(candidate) = dir {
        if candidate == package_home || !candidate.starts_with(package_home) {
            break;
        }
        let Ok(mut entries) = storage.read_dir(&candidate).await else {
            break;
        };
        if entries.next_entry().await?.is_some() {
            break;
        }
        storage.remove_dir_all(&candidate).await?;
        dir = candidate.parent().map(Path::to_path_buf);
    }
    Ok(())
}

/// Refuse unless every path this undo would touch holds what the revision being
/// undone committed there, or nothing where that revision has nothing.
///
/// The domain is the **union** of the tracked paths and the restore's
/// destinations: the restore writes rows of the revision being *restored*, so
/// it can reach paths this copy never tracked, and an unchecked destination is
/// one an untracked file gets overwritten at.
///
/// Compared by content, not by the lineage's cached [`PathState`] timestamps.
///
/// The message names no verb for discarding because there is none on a package
/// with no remote — <https://github.com/quiltdata/quilt-rs/issues/880>.
async fn refuse_unless_clean(
    lineage: &PackageLineage,
    storage: &(impl Storage + Sync),
    package_home: &Path,
    current_manifest: &Manifest,
    delta: &BTreeMap<PathBuf, RemoteChange>,
) -> Res {
    let destinations = delta.iter().filter_map(|(key, change)| match change {
        RemoteChange::Modified(_) | RemoteChange::Added(_) => Some(key),
        RemoteChange::Removed => None,
    });
    let mut to_check: BTreeSet<&PathBuf> = lineage.paths.keys().collect();
    to_check.extend(destinations);

    let mut unexpected: Vec<String> = Vec::new();
    for logical_key in to_check {
        let working_path = package_home.join(logical_key);
        let ok = match current_manifest.get_record(logical_key) {
            // Changed, absent, a directory, unreadable: all the same answer.
            Some(row) => matches!(
                refresh_hash(storage, &working_path, row.clone()).await,
                Ok(None)
            ),
            // Anything at a destination the undone revision lacks is
            // untracked. A directory is allowed: it exists only because
            // tracked files sit under it, each checked by the arm above.
            None => {
                !storage.exists(&working_path).await
                    || storage.read_dir(&working_path).await.is_ok()
            }
        };
        if !ok {
            unexpected.push(logical_key.display().to_string());
        }
    }

    if unexpected.is_empty() {
        return Ok(());
    }
    unexpected.sort();
    Err(Error::PackageOp(PackageOpError::Undo(format!(
        "{} would be overwritten, and does not hold this revision's committed content. \
         Commit or restore it by hand, then undo",
        unexpected.join(", ")
    ))))
}
