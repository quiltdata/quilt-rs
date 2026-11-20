use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use crate::Error;
use crate::InstallPathError;
use crate::LogicalKey;
use crate::Res;
use crate::error::PackageOpError;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::Host;

use super::install_paths::Mismatch;
use super::install_paths::cache_immutable_object;
use super::install_paths::refuse_clashing_local_paths;
use super::install_paths::refuse_if_present;

/// What [`install_to`] did, by logical key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallToReport {
    /// Files now in the destination folder.
    pub placed: Vec<PathBuf>,
    /// Rows whose bytes the remote no longer holds: a `physical_key` with no
    /// `versionId`, whose object a later revision replaced. Not placed.
    pub skipped: Vec<PathBuf>,
    /// Rows whose `physical_key` names an exact version whose bytes do not
    /// hash to the row, so the package records a wrong hash. Not placed.
    pub hash_mismatch: Vec<PathBuf>,
}

/// Installs rows of `manifest` into the folder `dest`, which can be anywhere.
///
/// There is no package home, no `InstalledPackage` and no lineage: nothing is
/// recorded about the folder, and nothing outside it is written. That includes
/// the manifest's metadata, rows' and folders' alike: only the files are
/// placed. Each file is fetched and checked against its row's hash first, by
/// the same check [`install_paths`](super::install_paths) makes before a file
/// enters the object store. A file that fails the check is left out and
/// reported, as `install_paths` does.
///
/// `only` picks rows by logical key; `None` installs every row. These are
/// refused before the first fetch:
///
/// - a key with no row;
/// - a row whose `physical_key` is not an `s3://` location, which this call
///   cannot fetch;
/// - two keys that name one local file;
/// - a folder between `dest` and a target that is a symbolic link, which would
///   put the file outside `dest`;
/// - a file or symbolic link already at a target, or a file where a target's
///   folder would go (in the destination itself or any folder between it and
///   the target).
///
/// Symbolic links between `dest` and a target are refused on purpose, though
/// quilt3 follows them: a link could put a file outside `dest`.
///
/// **Fetching is all-or-nothing; placing is not.** The fetched files are staged
/// in a hidden folder inside `dest`, removed when the call returns, so a failed
/// fetch places nothing. A call that is dropped before it returns, such as a
/// cancelled task, cannot clean up and leaves that `.quilt-install-*` folder
/// behind; it is safe to delete. Once every fetch has succeeded the staged
/// files are renamed into place one at a time. Just before the first rename
/// every target is checked to be absent again, as
/// [`install_paths`](super::install_paths) does, so a file that appeared during
/// the fetch is not overwritten, and no folder between `dest` and a target may
/// have become a symbolic link meanwhile. A rename that fails partway (a full
/// disk, a permission change) leaves the files already placed where they are:
/// they are not rolled back. Each target is checked once more right before its
/// own rename, which also stops a key that names a file placed earlier in the
/// call (two keys can name one file on a case-insensitive disk). A file
/// created at a target, or a folder swapped for a link, in the instant between
/// that check and its rename is not caught, since a rename replaces what it
/// finds.
///
/// A `dest` that did not exist, with any folders above it that did not, is
/// created for the install and removed again if the install fails before
/// placing anything. A folder that has become a symbolic link meanwhile is left
/// alone, with whatever it leads to.
///
/// `dest_registry`, which quilt3's `install` offers to put the package in
/// another registry, is not here: it depends on how local registries are
/// stored, which is not decided.
pub async fn install_to(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host: Option<&Host>,
    manifest: &Manifest,
    dest: &Path,
    only: Option<&[PathBuf]>,
) -> Res<InstallToReport> {
    let rows = select_rows(manifest, only)?;
    refuse_unfetchable_rows(&rows)?;
    refuse_clashing_local_paths(&rows)?;
    refuse_blocked_targets(storage, &rows, dest).await?;

    let created = missing_folders(storage, &rows, dest).await;
    let staging = dest.join(format!(".quilt-install-{}", uuid::Uuid::new_v4()));
    let installed = async {
        storage.create_dir_all(dest).await?;
        storage.create_dir_all(&staging).await?;
        fetch_and_place(storage, remote, host, &rows, dest, &staging).await
    }
    .await;
    // The staging folder is this call's own, so removing it whole is safe.
    let _ = storage.remove_dir_all(&staging).await;
    let report = match installed {
        Ok(report) => report,
        Err(err) => {
            for dir in &created {
                // A folder swapped for a link during the call is not this
                // call's to remove, nor is anything the link leads through.
                if under_a_link(storage, dir, dest).await {
                    continue;
                }
                // Fails, and keeps the folder, if anything is in it.
                let _ = storage.remove_dir(dir).await;
            }
            return Err(err);
        }
    };
    info!(
        "✔️ Installed {} files to {}, skipped {}, hash mismatch {}",
        report.placed.len(),
        dest.display(),
        report.skipped.len(),
        report.hash_mismatch.len()
    );
    Ok(report)
}

/// Whether `dir`, or a folder above it below `dest`, is a symbolic link. `dest`
/// and what is above it are not asked: a link there is the caller's choice.
async fn under_a_link(storage: &(impl Storage + Sync), dir: &Path, dest: &Path) -> bool {
    for folder in dir
        .ancestors()
        .take_while(|folder| folder.starts_with(dest))
    {
        if folder != dest && storage.is_symlink(folder).await {
            return true;
        }
    }
    false
}

/// The folders this install will make, deepest first: `dest` and the folders
/// above it, and the folders between `dest` and each target, that do not exist
/// yet.
async fn missing_folders(
    storage: &(impl Storage + Sync),
    rows: &[ManifestRow],
    dest: &Path,
) -> Vec<PathBuf> {
    let mut candidates: BTreeSet<PathBuf> = dest.ancestors().map(Path::to_path_buf).collect();
    for row in rows {
        candidates.extend(
            row.logical_key
                .join_to(dest)
                .ancestors()
                .skip(1)
                .take_while(|ancestor| ancestor.starts_with(dest))
                .map(Path::to_path_buf),
        );
    }
    let mut missing = Vec::new();
    for dir in candidates {
        if !dir.as_os_str().is_empty() && !storage.exists(&dir).await {
            missing.push(dir);
        }
    }
    missing.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
    missing
}

/// Refuses a target that is a symbolic link, dangling or not: a rename would
/// replace it, and `exists` follows links so it cannot see one that dangles.
pub(super) async fn refuse_linked_targets<'a>(
    storage: &(impl Storage + Sync),
    targets: impl Iterator<Item = (PathBuf, &'a LogicalKey)>,
) -> Res {
    let mut linked = Vec::new();
    for (target, key) in targets {
        if storage.is_symlink(&target).await {
            linked.push(key.to_path_buf());
        }
    }
    if linked.is_empty() {
        return Ok(());
    }
    linked.sort();
    Err(Error::InstallPath(InstallPathError::LocalFileExists(
        linked,
    )))
}

fn select_rows(manifest: &Manifest, only: Option<&[PathBuf]>) -> Res<Vec<ManifestRow>> {
    let Some(only) = only else {
        return Ok(manifest.rows.clone());
    };
    let by_key: HashMap<&Path, &ManifestRow> = manifest
        .rows
        .iter()
        .map(|row| (row.logical_key.as_path(), row))
        .collect();
    only.iter()
        .map(|path| {
            by_key
                .get(LogicalKey::new(path.as_path())?.as_path())
                .map(|row| (*row).clone())
                .ok_or_else(|| Error::InstallPath(InstallPathError::NotInRevision(path.clone())))
        })
        .collect()
}

/// A row with no `s3://` key has nowhere to be fetched from.
fn refuse_unfetchable_rows(rows: &[ManifestRow]) -> Res {
    match rows
        .iter()
        .find(|row| !row.physical_key.starts_with("s3://"))
    {
        Some(row) => Err(Error::PackageOp(PackageOpError::Package(format!(
            "{} cannot be installed: its physical key {:?} is not an s3:// location",
            row.logical_key, row.physical_key
        )))),
        None => Ok(()),
    }
}

/// Whether `path` holds something other than a folder this install can put files
/// in: a file, or anything else that cannot be listed.
pub(crate) async fn blocks_a_folder(storage: &(impl Storage + Sync), path: &Path) -> bool {
    storage.exists(path).await
        && (storage.is_file(path).await || storage.read_dir(path).await.is_err())
}

/// The folders between `dest` and the targets of `keys`, each once, with the
/// first key it holds. `dest` itself is included; it may be a link or a
/// folder that does not exist yet, which the checks below allow.
fn folders_above<'a>(
    dest: &Path,
    keys: impl Iterator<Item = &'a LogicalKey>,
) -> BTreeMap<PathBuf, &'a LogicalKey> {
    let mut folders = BTreeMap::new();
    for key in keys {
        for ancestor in key
            .join_to(dest)
            .ancestors()
            .skip(1)
            .take_while(|ancestor| ancestor.starts_with(dest))
        {
            folders.entry(ancestor.to_path_buf()).or_insert(key);
        }
    }
    folders
}

/// Refuses a key whose folder between `dest` and its target is a symbolic
/// link, which would put the file outside `dest`. `dest` itself may be a link:
/// that is the caller's choice.
pub(crate) async fn refuse_linked_ancestors<'a>(
    storage: &(impl Storage + Sync),
    dest: &Path,
    keys: impl Iterator<Item = &'a LogicalKey>,
) -> Res {
    // Shallowest first within one key's chain of folders; across keys the
    // order is alphabetical, so the link named is the first in that order.
    for (ancestor, key) in folders_above(dest, keys) {
        if ancestor != dest && storage.is_symlink(&ancestor).await {
            return Err(Error::PackageOp(PackageOpError::Package(format!(
                "{key} cannot be installed: {} is a symbolic link, which would put it outside the destination",
                ancestor.strip_prefix(dest).unwrap_or(&ancestor).display()
            ))));
        }
    }
    Ok(())
}

/// Refuses a target that already holds a file, and a target whose folder, or
/// any folder above it inside `dest`, is held by a file.
async fn refuse_blocked_targets(
    storage: &(impl Storage + Sync),
    rows: &[ManifestRow],
    dest: &Path,
) -> Res {
    refuse_linked_ancestors(storage, dest, rows.iter().map(|row| &row.logical_key)).await?;
    // Each folder is looked at once, however many targets are in it.
    let mut blocked = Vec::new();
    for ancestor in folders_above(dest, rows.iter().map(|row| &row.logical_key)).into_keys() {
        if blocks_a_folder(storage, &ancestor).await {
            let shown = ancestor.strip_prefix(dest).unwrap_or(&ancestor);
            let shown = if shown.as_os_str().is_empty() {
                dest
            } else {
                shown
            };
            blocked.push(shown.to_path_buf());
        }
    }
    if !blocked.is_empty() {
        return Err(Error::InstallPath(InstallPathError::LocalFileExists(
            blocked,
        )));
    }
    refuse_linked_targets(
        storage,
        rows.iter()
            .map(|row| (row.logical_key.join_to(dest), &row.logical_key)),
    )
    .await?;
    refuse_if_present(
        storage,
        rows.iter()
            .map(|row| (row.logical_key.join_to(dest), &row.logical_key)),
    )
    .await
}

async fn fetch_and_place(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host: Option<&Host>,
    rows: &[ManifestRow],
    dest: &Path,
    staging: &Path,
) -> Res<InstallToReport> {
    let mut report = InstallToReport::default();
    let mut staged = Vec::new();
    for row in rows {
        let held = staging.join(uuid::Uuid::new_v4().to_string());
        let fetching = staging.join(uuid::Uuid::new_v4().to_string());
        storage.create_dir_all(&fetching).await?;
        match cache_immutable_object(storage, remote, host, &held, row, &fetching).await? {
            None => staged.push((held, row.logical_key.join_to(dest), row)),
            Some(Mismatch::Replaced) => report.skipped.push(row.logical_key.to_path_buf()),
            Some(Mismatch::WrongHash) => report.hash_mismatch.push(row.logical_key.to_path_buf()),
        }
    }
    // A file may have appeared, or a folder become a link, during the fetches:
    // check again, as `install_paths` does, before the first rename.
    refuse_linked_ancestors(
        storage,
        dest,
        staged.iter().map(|(_, _, row)| &row.logical_key),
    )
    .await?;
    refuse_linked_targets(
        storage,
        staged
            .iter()
            .map(|(_, target, row)| (target.clone(), &row.logical_key)),
    )
    .await?;
    refuse_if_present(
        storage,
        staged
            .iter()
            .map(|(_, target, row)| (target.as_path(), &row.logical_key)),
    )
    .await?;
    for (held, target, row) in staged {
        // Again, right before this rename: on a case-insensitive disk two keys
        // can name one file, and the first rename has just made the second
        // target exist.
        refuse_if_present(storage, std::iter::once((&target, &row.logical_key))).await?;
        if let Some(parent) = target.parent() {
            storage.create_dir_all(parent).await?;
        }
        storage.rename(&held, &target).await?;
        debug!("✔️ Placed {}", target.display());
        report.placed.push(row.logical_key.to_path_buf());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_s3::primitives::ByteStream;
    use test_log::test;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::manifest::RawNumbers;
    use quilt_uri::S3Uri;

    /// The SHA-256 multihash of `body`, as a row's hash: a fetched object is
    /// verified against its row, so a row the test installs must name its
    /// body's real hash.
    fn row(name: &str, body: &[u8], physical_key: &str) -> ManifestRow {
        use sha2::Digest;
        let hash = multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body))
            .expect("multihash");
        ManifestRow {
            logical_key: LogicalKey::new(name).expect("key"),
            physical_key: physical_key.to_string(),
            hash: hash.try_into().expect("hash"),
            size: body.len() as u64,
            meta: None,
            meta_numbers: RawNumbers::default(),
        }
    }

    async fn put(remote: &MockRemote, uri: &str, body: &[u8]) -> Res {
        remote
            .put_object(None, &S3Uri::try_from(uri)?, body.to_vec())
            .await
    }

    fn manifest(rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            rows,
            ..Manifest::default()
        }
    }

    async fn read(storage: &MockStorage, path: &Path) -> Res<String> {
        let stream = storage.read_byte_stream(path).await?;
        Ok(String::from_utf8(stream.collect().await?.to_vec()).expect("utf-8"))
    }

    #[test(tokio::test)]
    async fn files_land_in_any_folder_with_nothing_recorded() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        put(&remote, "s3://b/p/sub/b.txt?versionId=v1", b"beta").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("sub/b.txt", b"beta", "s3://b/p/sub/b.txt?versionId=v1"),
        ]);
        let dest = std::path::absolute("/anywhere/at/all")?;

        let report = install_to(&storage, &remote, None, &manifest, &dest, None).await?;

        assert_eq!(
            report.placed,
            vec![PathBuf::from("a.txt"), PathBuf::from("sub/b.txt")]
        );
        assert_eq!(read(&storage, &dest.join("a.txt")).await?, "alpha");
        assert_eq!(read(&storage, &dest.join("sub/b.txt")).await?, "beta");
        // Only the files: no `.quilt` bookkeeping, and the staging folder is gone.
        let mut names = Vec::new();
        let mut entries = storage.read_dir(&dest).await?;
        while let Some(entry) = entries.next_entry().await? {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        names.sort();
        assert_eq!(names, vec!["a.txt", "sub"]);
        Ok(())
    }

    #[test(tokio::test)]
    async fn only_the_named_rows_are_installed() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        put(&remote, "s3://b/p/c.txt?versionId=v1", b"gamma").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("c.txt", b"gamma", "s3://b/p/c.txt?versionId=v1"),
        ]);
        let dest = std::path::absolute("/dest")?;

        let report = install_to(
            &storage,
            &remote,
            None,
            &manifest,
            &dest,
            Some(&[PathBuf::from("c.txt")]),
        )
        .await?;

        assert_eq!(report.placed, vec![PathBuf::from("c.txt")]);
        assert!(!storage.exists(dest.join("a.txt")).await);
        let missing = install_to(
            &storage,
            &remote,
            None,
            &manifest,
            &dest,
            Some(&[PathBuf::from("nope.txt")]),
        )
        .await;
        assert!(
            matches!(
                missing,
                Err(Error::InstallPath(InstallPathError::NotInRevision(_)))
            ),
            "{missing:?}"
        );
        Ok(())
    }

    /// Bytes that do not hash to the row are never placed, and a version-pinned
    /// row says its recorded hash is wrong, as `install_paths` does.
    #[test(tokio::test)]
    async fn bytes_that_do_not_match_their_row_are_not_placed() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/good.txt?versionId=v1", b"good").await?;
        put(&remote, "s3://b/p/pinned.txt?versionId=v1", b"tampered").await?;
        put(&remote, "s3://b/p/replaced.txt", b"newer").await?;
        let manifest = manifest(vec![
            row("good.txt", b"good", "s3://b/p/good.txt?versionId=v1"),
            row(
                "pinned.txt",
                b"original",
                "s3://b/p/pinned.txt?versionId=v1",
            ),
            row("replaced.txt", b"older", "s3://b/p/replaced.txt"),
        ]);
        let dest = std::path::absolute("/dest")?;

        let report = install_to(&storage, &remote, None, &manifest, &dest, None).await?;

        assert_eq!(report.placed, vec![PathBuf::from("good.txt")]);
        assert_eq!(report.hash_mismatch, vec![PathBuf::from("pinned.txt")]);
        assert_eq!(report.skipped, vec![PathBuf::from("replaced.txt")]);
        assert!(!storage.exists(dest.join("pinned.txt")).await);
        assert!(!storage.exists(dest.join("replaced.txt")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_file_already_in_the_folder_is_refused_before_any_fetch() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        put(&remote, "s3://b/p/b.txt?versionId=v1", b"beta").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("b.txt", b"beta", "s3://b/p/b.txt?versionId=v1"),
        ]);
        let dest = std::path::absolute("/dest")?;
        storage
            .write_byte_stream(dest.join("b.txt"), ByteStream::from_static(b"mine"))
            .await?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(
            matches!(
                &installed,
                Err(Error::InstallPath(InstallPathError::LocalFileExists(paths)))
                    if paths == &vec![PathBuf::from("b.txt")]
            ),
            "{installed:?}"
        );
        assert_eq!(read(&storage, &dest.join("b.txt")).await?, "mine");
        assert!(!storage.exists(dest.join("a.txt")).await);
        assert_eq!(remote.get_object_count("s3://b/p/a.txt?versionId=v1"), 0);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_failed_fetch_places_nothing() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        // `gone.txt` is not on the remote at all.
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("gone.txt", b"x", "s3://b/p/gone.txt?versionId=v1"),
        ]);
        let dest = std::path::absolute("/dest")?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(installed.is_err());
        assert!(!storage.exists(dest.join("a.txt")).await);
        assert!(
            !storage.exists(&dest).await,
            "a destination the install made is removed when it fails"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_failed_fetch_removes_every_folder_it_made_above_the_destination() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        let manifest = manifest(vec![row(
            "gone.txt",
            b"x",
            "s3://b/p/gone.txt?versionId=v1",
        )]);
        let kept = std::path::absolute("/kept")?;
        storage.create_dir_all(&kept).await?;
        let dest = kept.join("made/also/dest");

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(installed.is_err());
        assert!(!storage.exists(kept.join("made")).await);
        assert!(storage.exists(&kept).await, "a folder that was there stays");
        Ok(())
    }

    /// Cleanup removes a folder only if it is empty, so a file that arrived in it
    /// is never deleted along with it.
    #[test(tokio::test)]
    async fn removing_a_folder_that_holds_a_file_fails_and_keeps_the_file() -> Res {
        let storage = MockStorage::default();
        let dir = std::path::absolute("/made")?;
        storage.create_dir_all(&dir).await?;
        storage
            .write_byte_stream(dir.join("late.txt"), ByteStream::from_static(b"late"))
            .await?;

        assert!(storage.remove_dir(&dir).await.is_err());

        assert!(storage.exists(dir.join("late.txt")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_failed_fetch_keeps_a_destination_that_was_already_there() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        let manifest = manifest(vec![row(
            "gone.txt",
            b"x",
            "s3://b/p/gone.txt?versionId=v1",
        )]);
        let dest = std::path::absolute("/dest")?;
        storage.create_dir_all(&dest).await?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(installed.is_err());
        let mut entries = storage.read_dir(&dest).await?;
        assert!(entries.next_entry().await?.is_none(), "staging was removed");
        Ok(())
    }

    /// A link between the destination and a target would put the file somewhere
    /// else, past the checks made on the paths as written.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_symlinked_folder_is_refused_before_any_fetch() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/sub/b.txt?versionId=v1", b"beta").await?;
        let manifest = manifest(vec![row(
            "sub/b.txt",
            b"beta",
            "s3://b/p/sub/b.txt?versionId=v1",
        )]);
        let dest = std::path::absolute("/dest")?;
        let outside = std::path::absolute("/outside")?;
        storage.create_dir_all(&dest).await?;
        storage.create_dir_all(&outside).await?;
        let root = storage.temp_dir.path();
        std::os::unix::fs::symlink(root.join("outside"), root.join("dest/sub"))?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        let message = installed.expect_err("a link is refused").to_string();
        assert!(message.contains("symbolic link"), "{message}");
        assert!(!storage.exists(outside.join("b.txt")).await);
        assert_eq!(
            remote.get_object_count("s3://b/p/sub/b.txt?versionId=v1"),
            0
        );
        Ok(())
    }

    /// The same refusal holds when the storage is shared through an `Arc`.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_symlinked_folder_is_refused_through_an_arc_storage() -> Res {
        let remote = MockRemote::default();
        let storage = std::sync::Arc::new(MockStorage::default());
        let manifest = manifest(vec![row(
            "sub/b.txt",
            b"beta",
            "s3://b/p/sub/b.txt?versionId=v1",
        )]);
        let dest = std::path::absolute("/dest")?;
        storage.create_dir_all(&dest).await?;
        storage
            .create_dir_all(std::path::absolute("/outside")?)
            .await?;
        let root = storage.temp_dir.path();
        std::os::unix::fs::symlink(root.join("outside"), root.join("dest/sub"))?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(installed.is_err_and(|err| err.to_string().contains("symbolic link")));
        Ok(())
    }

    /// A link already at a target, even one that points nowhere, is not
    /// replaced: `exists` cannot see a dangling one.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_dangling_symlink_at_a_target_is_refused_before_any_fetch() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        let manifest = manifest(vec![row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1")]);
        let dest = std::path::absolute("/dest")?;
        storage.create_dir_all(&dest).await?;
        let root = storage.temp_dir.path();
        std::os::unix::fs::symlink(root.join("nowhere"), root.join("dest/a.txt"))?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(matches!(
            installed,
            Err(Error::InstallPath(InstallPathError::LocalFileExists(_)))
        ));
        assert!(storage.is_symlink(dest.join("a.txt")).await);
        assert_eq!(remote.get_object_count("s3://b/p/a.txt?versionId=v1"), 0);
        Ok(())
    }

    /// The folders to remove after a failure include those between the
    /// destination and a target, and leave out any that already exist.
    #[test(tokio::test)]
    async fn the_folders_an_install_will_make_include_each_targets_parents() -> Res {
        let storage = MockStorage::default();
        let kept = std::path::absolute("/kept")?;
        storage.create_dir_all(kept.join("dest/there")).await?;
        let rows = vec![
            row("a.txt", b"a", "s3://b/p/a.txt?versionId=v1"),
            row("sub/deep/b.txt", b"b", "s3://b/p/b.txt?versionId=v1"),
            row("there/c.txt", b"c", "s3://b/p/c.txt?versionId=v1"),
        ];

        let made = missing_folders(&storage, &rows, &kept.join("dest")).await;

        assert_eq!(
            made,
            vec![kept.join("dest/sub/deep"), kept.join("dest/sub")]
        );
        let nested = missing_folders(&storage, &rows, &kept.join("new/dest")).await;
        assert_eq!(nested[0], kept.join("new/dest/sub/deep"));
        assert_eq!(nested.last(), Some(&kept.join("new")));
        assert!(!nested.contains(&kept));
        Ok(())
    }

    /// The destination itself is the caller's choice, a link or not.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_destination_that_is_a_symlink_is_allowed() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        let manifest = manifest(vec![row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1")]);
        let real = std::path::absolute("/real")?;
        storage.create_dir_all(&real).await?;
        let root = storage.temp_dir.path();
        std::os::unix::fs::symlink(root.join("real"), root.join("linked"))?;
        let dest = std::path::absolute("/linked")?;

        let report = install_to(&storage, &remote, None, &manifest, &dest, None).await?;

        assert_eq!(report.placed.len(), 1);
        assert!(storage.exists(real.join("a.txt")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_file_where_a_folder_would_go_is_refused_before_any_fetch() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        put(&remote, "s3://b/p/sub/deep/b.txt?versionId=v1", b"beta").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row(
                "sub/deep/b.txt",
                b"beta",
                "s3://b/p/sub/deep/b.txt?versionId=v1",
            ),
        ]);
        let dest = std::path::absolute("/dest")?;
        storage
            .write_byte_stream(dest.join("sub"), ByteStream::from_static(b"a file"))
            .await?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(
            matches!(
                &installed,
                Err(Error::InstallPath(InstallPathError::LocalFileExists(paths)))
                    if paths == &vec![PathBuf::from("sub")]
            ),
            "{installed:?}"
        );
        assert_eq!(remote.get_object_count("s3://b/p/a.txt?versionId=v1"), 0);
        assert!(!storage.exists(dest.join("a.txt")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_destination_that_is_a_file_is_refused() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        let manifest = manifest(vec![row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1")]);
        let dest = std::path::absolute("/dest")?;
        storage
            .write_byte_stream(&dest, ByteStream::from_static(b"a file"))
            .await?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(
            matches!(
                &installed,
                Err(Error::InstallPath(InstallPathError::LocalFileExists(_)))
            ),
            "{installed:?}"
        );
        assert_eq!(remote.get_object_count("s3://b/p/a.txt?versionId=v1"), 0);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_row_that_is_not_on_s3_is_refused_before_any_fetch() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("local.txt", b"x", "file:///somewhere/local.txt"),
        ]);
        let dest = std::path::absolute("/dest")?;

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(
            matches!(&installed, Err(Error::PackageOp(PackageOpError::Package(message))) if message.contains("local.txt")),
            "{installed:?}"
        );
        assert_eq!(remote.get_object_count("s3://b/p/a.txt?versionId=v1"), 0);
        assert!(!storage.exists(&dest).await);
        Ok(())
    }

    /// A file that appears while the fetches run is the user's own: the check
    /// just before the first rename refuses, and nothing is placed.
    #[test(tokio::test)]
    async fn a_file_that_appears_during_the_fetch_is_not_overwritten() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        put(&remote, "s3://b/p/b.txt?versionId=v1", b"beta").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("b.txt", b"beta", "s3://b/p/b.txt?versionId=v1"),
        ]);
        let dest = std::path::absolute("/dest")?;
        let gate = remote.park("s3://b/p/b.txt?versionId=v1");

        let install = install_to(&storage, &remote, None, &manifest, &dest, None);
        let meddle = async {
            gate.arrived().await;
            storage
                .write_byte_stream(dest.join("a.txt"), ByteStream::from_static(b"mine"))
                .await?;
            gate.release();
            Ok::<(), Error>(())
        };
        let (installed, meddled) = tokio::join!(install, meddle);
        meddled?;

        assert!(
            matches!(
                &installed,
                Err(Error::InstallPath(InstallPathError::LocalFileExists(paths)))
                    if paths == &vec![PathBuf::from("a.txt")]
            ),
            "{installed:?}"
        );
        assert_eq!(read(&storage, &dest.join("a.txt")).await?, "mine");
        assert!(!storage.exists(dest.join("b.txt")).await);
        Ok(())
    }

    /// A folder that becomes a link while the fetches run is not followed: the
    /// check just before the first rename refuses, nothing is written through
    /// the link, and the cleanup after the refusal removes nothing through it.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_folder_swapped_for_a_link_during_the_fetch_is_not_followed() -> Res {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        put(&remote, "s3://b/p/sub/deep/a.txt?versionId=v1", b"alpha").await?;
        let manifest = manifest(vec![row(
            "sub/deep/a.txt",
            b"alpha",
            "s3://b/p/sub/deep/a.txt?versionId=v1",
        )]);
        let dest = std::path::absolute("/dest")?;
        let outside = std::path::absolute("/outside")?;
        storage.create_dir_all(outside.join("deep")).await?;
        let gate = remote.park("s3://b/p/sub/deep/a.txt?versionId=v1");

        let install = install_to(&storage, &remote, None, &manifest, &dest, None);
        let meddle = async {
            gate.arrived().await;
            let root = storage.temp_dir.path();
            std::os::unix::fs::symlink(root.join("outside"), root.join("dest/sub"))?;
            gate.release();
            Ok::<(), Error>(())
        };
        let (installed, meddled) = tokio::join!(install, meddle);
        meddled?;

        let message = installed.expect_err("a link is refused").to_string();
        assert!(message.contains("symbolic link"), "{message}");
        assert!(!storage.exists(outside.join("deep/a.txt")).await);
        // Cleanup after the refusal did not follow the link either.
        assert!(storage.exists(outside.join("deep")).await);
        Ok(())
    }

    /// Each target is checked again right before its own rename. Placing the
    /// first file makes the second target appear, as a case-insensitive disk
    /// does for two keys that differ only in case; the second is refused, not
    /// replaced, and the first stays where it was placed.
    #[test(tokio::test)]
    async fn a_target_made_by_an_earlier_rename_is_not_overwritten() -> Res {
        let remote = MockRemote::default();
        let mock = MockStorage::default();
        put(&remote, "s3://b/p/a.txt?versionId=v1", b"alpha").await?;
        put(&remote, "s3://b/p/b.txt?versionId=v1", b"beta").await?;
        let manifest = manifest(vec![
            row("a.txt", b"alpha", "s3://b/p/a.txt?versionId=v1"),
            row("b.txt", b"beta", "s3://b/p/b.txt?versionId=v1"),
        ]);
        let dest = std::path::absolute("/dest")?;
        let storage = PlantAfterRename {
            inner: &mock,
            after: dest.join("a.txt"),
            plant: dest.join("b.txt"),
        };

        let installed = install_to(&storage, &remote, None, &manifest, &dest, None).await;

        assert!(
            matches!(
                &installed,
                Err(Error::InstallPath(InstallPathError::LocalFileExists(paths)))
                    if paths == &vec![PathBuf::from("b.txt")]
            ),
            "{installed:?}"
        );
        assert_eq!(read(&mock, &dest.join("b.txt")).await?, "planted");
        assert_eq!(read(&mock, &dest.join("a.txt")).await?, "alpha");
        Ok(())
    }

    /// [`MockStorage`] that writes a file at `plant` once a file is renamed
    /// onto `after`: a rename whose effect shows up at another path, which an
    /// in-process test cannot otherwise stage between two renames.
    struct PlantAfterRename<'a> {
        inner: &'a MockStorage,
        after: PathBuf,
        plant: PathBuf,
    }

    impl Storage for PlantAfterRename<'_> {
        async fn copy(
            &self,
            from: impl AsRef<Path> + Send,
            to: impl AsRef<Path> + Send,
        ) -> Res<u64> {
            self.inner.copy(from, to).await
        }
        async fn create_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.create_dir_all(path).await
        }
        async fn remove_dir(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.remove_dir(path).await
        }
        async fn is_symlink(&self, path: impl AsRef<Path> + Send) -> bool {
            self.inner.is_symlink(path).await
        }
        async fn create_file(&self, path: impl AsRef<Path>) -> Res<tokio::fs::File> {
            self.inner.create_file(path).await
        }
        async fn exists(&self, path: impl AsRef<Path>) -> bool {
            self.inner.exists(path).await
        }
        async fn is_file(&self, path: impl AsRef<Path>) -> bool {
            self.inner.is_file(path).await
        }
        async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<jiff::Timestamp> {
            self.inner.modified_timestamp(path).await
        }
        async fn open_file(&self, path: impl AsRef<Path> + Send) -> Res<tokio::fs::File> {
            self.inner.open_file(path).await
        }
        async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
            self.inner.read_byte_stream(path).await
        }
        async fn read_dir(&self, path: impl AsRef<Path> + Send + Sync) -> Res<tokio::fs::ReadDir> {
            self.inner.read_dir(path).await
        }
        async fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.remove_dir_all(path).await
        }
        async fn remove_file(&self, path: impl AsRef<Path> + Send) -> Result<(), std::io::Error> {
            self.inner.remove_file(path).await
        }
        async fn rename(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res {
            let onto = to.as_ref() == self.after;
            self.inner.rename(from, to).await?;
            if onto {
                self.inner
                    .write_byte_stream(&self.plant, ByteStream::from_static(b"planted"))
                    .await?;
            }
            Ok(())
        }
        async fn write_byte_stream(
            &self,
            path: impl AsRef<Path> + Send + Sync,
            body: ByteStream,
        ) -> Res {
            self.inner.write_byte_stream(path, body).await
        }
        async fn create_private_dir(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.create_private_dir(path).await
        }
        async fn write_private_byte_stream(
            &self,
            path: impl AsRef<Path> + Send + Sync,
            body: ByteStream,
        ) -> Res {
            self.inner.write_private_byte_stream(path, body).await
        }
        async fn lock_exclusive(
            &self,
            path: impl AsRef<Path> + Send,
        ) -> Res<crate::io::storage::LockGuard> {
            self.inner.lock_exclusive(path).await
        }
        async fn try_lock_exclusive(
            &self,
            path: impl AsRef<Path> + Send,
        ) -> Res<Option<crate::io::storage::LockGuard>> {
            self.inner.try_lock_exclusive(path).await
        }
    }
}
