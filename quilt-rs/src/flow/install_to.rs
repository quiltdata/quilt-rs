use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use crate::Error;
use crate::InstallPathError;
use crate::LogicalKey;
use crate::Res;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::Host;

use super::install_paths::Mismatch;
use super::install_paths::cache_immutable_object;
use super::install_paths::refuse_clashing_local_paths;

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
/// recorded about the folder, and nothing outside it is written. Each file is
/// fetched and checked against its row's hash before it is placed, by the same
/// check [`install_paths`](super::install_paths) makes before a file enters the
/// object store. A file that fails the check is left out and reported, as
/// `install_paths` does.
///
/// `only` picks rows by logical key; `None` installs every row. A key with no
/// row is refused, as is any file already in the destination, and so are two keys
/// that name one local file. Every refusal comes before the first fetch. The
/// fetched files are staged in a hidden folder inside `dest`, which is removed
/// on every exit, and renamed into place only once all of them are fetched, so a
/// failed fetch places nothing.
///
/// Every selected row must have an `s3://` `physical_key`.
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
    refuse_clashing_local_paths(&rows)?;
    let mut existing = Vec::new();
    for row in &rows {
        if storage.exists(row.logical_key.join_to(dest)).await {
            existing.push(row.logical_key.to_path_buf());
        }
    }
    if !existing.is_empty() {
        return Err(Error::InstallPath(InstallPathError::LocalFileExists(
            existing,
        )));
    }

    storage.create_dir_all(dest).await?;
    let staging = dest.join(format!(".quilt-install-{}", uuid::Uuid::new_v4()));
    storage.create_dir_all(&staging).await?;
    let installed = fetch_and_place(storage, remote, host, &rows, dest, &staging).await;
    let _ = storage.remove_dir_all(&staging).await;
    let report = installed?;
    info!(
        "✔️ Installed {} files to {}, skipped {}, hash mismatch {}",
        report.placed.len(),
        dest.display(),
        report.skipped.len(),
        report.hash_mismatch.len()
    );
    Ok(report)
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
            None => staged.push((held, row)),
            Some(Mismatch::Replaced) => report.skipped.push(row.logical_key.to_path_buf()),
            Some(Mismatch::WrongHash) => report.hash_mismatch.push(row.logical_key.to_path_buf()),
        }
    }
    for (held, row) in staged {
        let target = row.logical_key.join_to(dest);
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
        let mut entries = storage.read_dir(&dest).await?;
        assert!(entries.next_entry().await?.is_none(), "staging was removed");
        Ok(())
    }
}
