//! Replacing files of a working folder with a revision's, all or none: what
//! `reset_to_latest` and `undo_commit` write into a folder.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use crate::LogicalKey;
use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::Entry;
use crate::dataspace::read::STAGING_PREFIX;
use crate::dataspace::read::write_metadata;
use crate::dataspace::store::Store;
use crate::dataspace::sync::stage;
use crate::flow::refuse_linked_ancestors;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use tracing::warn;

/// The files [`replace`] could not fetch whole, so it changed nothing.
pub(crate) struct Unfetched(pub(crate) Vec<PathBuf>);

/// Replaces, in `entry`'s working folder, what stands at each key of `clear`
/// and of `place`: each key of `place` takes `target`'s file, the revision
/// `hash`, and a key only in `clear` is removed, with the folders that
/// leaves empty. The metadata file takes `metadata`'s text, or is removed
/// with `None`. Every file is fetched before the folder is touched, and
/// what was replaced is kept aside until the last is placed: one that fails
/// part-way puts the folder back. Nothing else in the folder is touched.
#[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
pub(crate) async fn replace(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    (hash, target): (&str, &Manifest),
    place: &BTreeSet<&LogicalKey>,
    clear: &BTreeSet<&LogicalKey>,
    metadata: Option<String>,
) -> Result<Result<(), Unfetched>, EntryError> {
    let folder = &entry.folder;
    storage.create_dir_all(folder).await?;
    let staging = folder.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let aside = folder.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let placing = Manifest {
        rows: target
            .rows
            .iter()
            .filter(|row| place.contains(&row.logical_key))
            .cloned()
            .collect(),
        ..Manifest::default()
    };
    // What could not be moved back stays in `aside`, the only copy.
    let mut restored = true;
    let replaced = async {
        let report = stage(storage, remote, store, entry, hash, &placing, &staging).await?;
        if !report.complete() {
            let mut unfetched = report.skipped;
            unfetched.extend(report.hash_mismatch);
            return Ok(Err(Unfetched(unfetched)));
        }
        refuse_linked_ancestors(storage, folder, place.union(clear).copied()).await?;
        let mut moves = Moves::default();
        let done = moves
            .run(storage, folder, &staging, &aside, place, clear, metadata)
            .await;
        if let Err(err) = done {
            restored = moves.undo(storage, folder).await;
            return Err(err);
        }
        Ok(Ok(()))
    }
    .await;
    let _ = storage.remove_dir_all(&staging).await;
    if restored {
        let _ = storage.remove_dir_all(&aside).await;
    } else {
        warn!(
            "Files that could not be put back are kept in {}",
            aside.display()
        );
    }
    replaced
}

/// What a replace moved in a working folder, so a failure can move it back.
#[derive(Default)]
struct Moves {
    /// Each path in the folder moved aside, with where it went.
    aside: Vec<(PathBuf, PathBuf)>,
    /// Paths placed in the folder.
    placed: Vec<PathBuf>,
    /// Whether the metadata file was written.
    metadata: bool,
}

impl Moves {
    /// Moves aside what stands at each key of `place` and `clear`, and the
    /// metadata file, pruning the folders that leaves empty, then renames
    /// each staged file of `place` into the folder and writes the metadata
    /// file's `text`, if any.
    #[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
    async fn run(
        &mut self,
        storage: &(impl Storage + Sync),
        folder: &Path,
        staging: &Path,
        aside: &Path,
        place: &BTreeSet<&LogicalKey>,
        clear: &BTreeSet<&LogicalKey>,
        metadata: Option<String>,
    ) -> Result<(), EntryError> {
        let places = place
            .union(clear)
            .map(|key| (key.join_to(folder), key.join_to(aside)))
            .chain(std::iter::once((
                folder.join(METADATA_FILE),
                aside.join(METADATA_FILE),
            )));
        for (path, kept) in places {
            // A folder where a key's file was is what files under it left.
            if storage.is_symlink(&path).await || storage.is_file(&path).await {
                if let Some(parent) = kept.parent() {
                    storage.create_dir_all(parent).await?;
                }
                storage.rename(&path, &kept).await?;
                self.aside.push((path, kept));
            }
        }
        // A file cannot be placed where a folder emptied above stands.
        for (path, _) in &self.aside {
            prune(storage, folder, path).await;
        }
        for key in place {
            let path = key.join_to(folder);
            if let Some(parent) = path.parent() {
                storage.create_dir_all(parent).await?;
            }
            storage.rename(key.join_to(staging), &path).await?;
            self.placed.push(path);
            crate::failpoint::stop("replace.placed")?;
        }
        if let Some(text) = metadata {
            write_metadata(storage, folder, text, None).await?;
            self.metadata = true;
        }
        Ok(())
    }

    /// Removes what was placed and moves back what was moved aside, and
    /// says whether all of it went back.
    async fn undo(&self, storage: &(impl Storage + Sync), folder: &Path) -> bool {
        let mut restored = true;
        let written = self.metadata.then(|| folder.join(METADATA_FILE));
        for path in self.placed.iter().chain(&written) {
            let _ = storage.remove_file(path).await;
            prune(storage, folder, path).await;
        }
        for (path, kept) in &self.aside {
            if let Some(parent) = path.parent() {
                let _ = storage.create_dir_all(parent).await;
            }
            restored &= storage.rename(kept, path).await.is_ok();
        }
        restored
    }
}

/// Removes each folder between `path` and `folder` that is empty, deepest
/// first.
async fn prune(storage: &(impl Storage + Sync), folder: &Path, path: &Path) {
    for dir in path
        .ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(folder) && *dir != folder)
    {
        if storage.remove_dir(dir).await.is_err() {
            return;
        }
    }
}
