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
/// the manifest's metadata, rows' and folders' alike: only the files are placed. Each file is
/// fetched and checked against its row's hash first, by the same check
/// [`install_paths`](super::install_paths) makes before a file enters the object
/// store. A file that fails the check is left out and reported, as
/// `install_paths` does.
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
/// - a file already at a target, or a file where a target's folder would go
///   (in the destination itself or any folder between it and the target).
///
/// **Fetching is all-or-nothing; placing is not.** The fetched files are staged
/// in a hidden folder inside `dest`, removed when the call returns, so a failed
/// fetch places nothing. A call that is dropped before it returns, such as a
/// cancelled task, cannot clean up and leaves that `.quilt-install-*` folder
/// behind; it is safe to delete. Once every fetch has succeeded the staged files are renamed
/// into place one at a time. Just before the first rename every target is
/// checked to be absent again, as [`install_paths`](super::install_paths) does,
/// so a file that appeared during the fetch is not overwritten. A rename that
/// fails partway (a full disk, a permission change) leaves the files already
/// placed where they are: they are not rolled back. A file created at a target
/// during the renames themselves, a window of milliseconds, is not caught, since
/// a rename replaces what it finds.
///
/// A `dest` that did not exist, with any folders above it that did not, is
/// created for the install and removed again if the install fails before
/// placing anything.
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

    let created = missing_folders(storage, dest).await;
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

/// `dest` and the folders above it that do not exist yet, deepest first: the ones
/// this install will make.
async fn missing_folders(storage: &(impl Storage + Sync), dest: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    for dir in dest.ancestors() {
        if storage.exists(dir).await {
            break;
        }
        missing.push(dir.to_path_buf());
    }
    missing
}

fn select_rows(manifest: &Manifest, only: Option<&[PathBuf]>) -> Res<Vec<ManifestRow>> {
    let Some(only) = only else {
        return Ok(manifest.rows.clone());
    };
    only.iter()
        .map(|path| {
            manifest
                .get_record(LogicalKey::new(path.as_path())?.as_path())
                .cloned()
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
async fn blocks_a_folder(storage: &(impl Storage + Sync), path: &Path) -> bool {
    storage.exists(path).await
        && (storage.is_file(path).await || storage.read_dir(path).await.is_err())
}

/// Refuses a target that already holds a file, and a target whose folder, or
/// any folder above it inside `dest`, is held by a file.
async fn refuse_blocked_targets(
    storage: &(impl Storage + Sync),
    rows: &[ManifestRow],
    dest: &Path,
) -> Res {
    let mut blocked = Vec::new();
    for row in rows {
        let target = row.logical_key.join_to(dest);
        for ancestor in target
            .ancestors()
            .skip(1)
            .take_while(|ancestor| ancestor.starts_with(dest))
        {
            if ancestor != dest && storage.is_symlink(ancestor).await {
                return Err(Error::PackageOp(PackageOpError::Package(format!(
                    "{} cannot be installed: {} is a symbolic link, which would put it outside the destination",
                    row.logical_key,
                    ancestor.strip_prefix(dest).unwrap_or(ancestor).display()
                ))));
            }
            if blocks_a_folder(storage, ancestor).await {
                let shown = ancestor.strip_prefix(dest).unwrap_or(ancestor);
                let shown = if shown.as_os_str().is_empty() {
                    dest
                } else {
                    shown
                };
                blocked.push(shown.to_path_buf());
            }
        }
    }
    if !blocked.is_empty() {
        blocked.sort();
        blocked.dedup();
        return Err(Error::InstallPath(InstallPathError::LocalFileExists(
            blocked,
        )));
    }
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
    // A file may have appeared during the fetches: check again, as
    // `install_paths` does, before the first rename.
    refuse_if_present(
        storage,
        staged
            .iter()
            .map(|(_, target, row)| (target.as_path(), &row.logical_key)),
    )
    .await?;
    for (held, target, row) in staged {
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
}
