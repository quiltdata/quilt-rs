use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use jiff::Timestamp;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::debug;
use tracing::warn;
use uuid::Uuid;

use crate::Error;
use crate::Res;
use crate::error::FsError;

use super::Storage;

/// A unique name for a temp file that names no target.
pub(crate) fn temp_name() -> String {
    format!(".tmp-{}", Uuid::new_v4())
}

#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;
#[cfg(unix)]
pub(super) const PRIVATE_FILE_MODE: u32 = 0o600;

/// Write `body` to a new file at `path` and sync to disk. A `private` file is
/// created owner-only, so it is never readable by others, not even empty.
async fn write_stream(path: &Path, mut body: ByteStream, private: bool) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        options.mode(PRIVATE_FILE_MODE);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path).await?;
    while let Some(bytes) = body.try_next().await.map_err(std::io::Error::other)? {
        file.write_all(&bytes).await?;
    }
    file.sync_all().await
}

/// The folder a write to `path` lands in. A bare file name's parent is the
/// empty path, which cannot be opened to sync it.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn failpoint(tag: Option<&str>, step: &str) {
    if let Some(tag) = tag {
        crate::failpoint::hit(format_args!("{tag}.{step}"));
    }
}

/// Write `body` to the temp file `temp_name` in `path`'s folder, so the rename
/// is atomic, then rename it to `path`.
///
/// On failure the temp file is cleaned up.
async fn atomic_write(
    path: &Path,
    temp_name: &Path,
    body: ByteStream,
    private: bool,
    sync_folder: bool,
    tag: Option<&str>,
) -> std::io::Result<()> {
    // A path would move the temp out of the folder, and the target's own name
    // would write over it in place: either way the rename is not atomic.
    if temp_name.file_name() != Some(temp_name.as_os_str())
        || path.file_name() == Some(temp_name.as_os_str())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "temp name {} is not a file name of its own",
                temp_name.display()
            ),
        ));
    }
    let parent = folder_of(path);
    fs::create_dir_all(parent).await?;
    let tmp = parent.join(temp_name);
    let cleanup = |_: &std::io::Error| {
        let _ = std::fs::remove_file(&tmp);
    };
    failpoint(tag, "before-temp");
    write_stream(&tmp, body, private)
        .await
        .inspect_err(cleanup)?;
    failpoint(tag, "after-temp");
    fs::rename(&tmp, path).await.inspect_err(cleanup)?;
    failpoint(tag, "after-rename");
    #[cfg(unix)]
    if sync_folder {
        sync_dir(parent).await;
    }
    #[cfg(not(unix))]
    let _ = sync_folder;
    Ok(())
}

/// Makes a rename into `dir` survive a power loss. Windows has no directory
/// fsync. A failure is logged, not returned: the new file is already in place.
#[cfg(unix)]
async fn sync_dir(dir: &Path) {
    let synced = async { fs::File::open(dir).await?.sync_all().await };
    match synced.await {
        Ok(()) => {}
        // Network and FUSE mounts refuse every folder sync: a warning would
        // repeat on each write.
        Err(err) if sync_unsupported(&err) => {
            debug!("Could not sync {}: {}", dir.display(), err);
        }
        Err(err) => warn!("❌ Could not sync {}: {}", dir.display(), err),
    }
}

/// Whether `err` is a file system's refusal to sync a folder at all. Matched by
/// code: the `ErrorKind` of `ENOTSUP` differs between Rust releases.
#[cfg(unix)]
fn sync_unsupported(err: &std::io::Error) -> bool {
    err.raw_os_error().is_some_and(|code| {
        [
            libc::EINVAL,
            libc::ENOTSUP,
            libc::EOPNOTSUPP,
            libc::ENOSYS,
            libc::ENOTTY,
        ]
        .contains(&code)
    })
}

pub(super) async fn write(path: &Path, body: ByteStream, private: bool, sync_folder: bool) -> Res {
    write_via(path, temp_name().as_ref(), body, private, sync_folder, None).await
}

async fn write_via(
    path: &Path,
    temp_name: &Path,
    body: ByteStream,
    private: bool,
    sync_folder: bool,
    tag: Option<&str>,
) -> Res {
    atomic_write(path, temp_name, body, private, sync_folder, tag)
        .await
        .map_err(|source| {
            Error::Fs(FsError::Write {
                path: path.to_path_buf(),
                source,
            })
        })
}

/// Limits a directory made before it was private to its owner. Best effort: a
/// file system that keeps no Unix modes refuses the change.
#[cfg(unix)]
async fn tighten_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::Permissions::from_mode(PRIVATE_DIR_MODE);
    if let Err(err) = fs::set_permissions(path, mode).await {
        warn!(
            "❌ Could not limit {} to its owner: {}",
            path.display(),
            err
        );
    }
}

pub(super) async fn create_private_dir(path: &Path) -> Res {
    let created = async {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(PRIVATE_DIR_MODE);
        match builder.create(path).await {
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if !fs::metadata(path).await.is_ok_and(|m| m.is_dir()) {
                    return Err(err);
                }
                #[cfg(unix)]
                tighten_dir(path).await;
                Ok(())
            }
            created => created,
        }
    };
    created.await.map_err(|source| {
        Error::Fs(FsError::DirectoryCreate {
            path: path.to_path_buf(),
            source,
        })
    })
}

/// Implementation of the `Storage` trait for the local filesystem
#[derive(Clone, Debug)]
pub struct LocalStorage {}

impl Storage for LocalStorage {
    async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Res<u64> {
        let from = from.as_ref();
        let to = to.as_ref();
        fs::copy(from, to).await.map_err(|e| {
            Error::Fs(FsError::Copy {
                from: from.to_path_buf(),
                to: to.to_path_buf(),
                source: e,
            })
        })
    }

    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Res {
        let path = path.as_ref();
        fs::create_dir_all(path).await.map_err(|e| {
            Error::Fs(FsError::DirectoryCreate {
                path: path.to_path_buf(),
                source: e,
            })
        })
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
        let path = path.as_ref();
        fs::File::create(path).await.map_err(|e| {
            Error::Fs(FsError::Write {
                path: path.to_path_buf(),
                source: e,
            })
        })
    }

    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        fs::metadata(path).await.is_ok()
    }

    async fn is_file(&self, path: impl AsRef<Path>) -> bool {
        fs::metadata(path).await.is_ok_and(|m| m.is_file())
    }

    async fn remove_dir(&self, path: impl AsRef<Path> + Send) -> Res {
        Ok(fs::remove_dir(path.as_ref()).await?)
    }

    async fn is_symlink(&self, path: impl AsRef<Path> + Send) -> bool {
        fs::symlink_metadata(path.as_ref())
            .await
            .is_ok_and(|m| m.file_type().is_symlink())
    }

    async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<Timestamp> {
        let modified = fs::metadata(path).await.map(|m| m.modified())??;
        Ok(crate::time::from_system(modified)?)
    }

    async fn open_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
        let path = path.as_ref();
        fs::File::open(path).await.map_err(|e| {
            Error::Fs(FsError::Read {
                path: path.to_path_buf(),
                source: e,
            })
        })
    }

    async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
        let path = path.as_ref();
        ByteStream::from_path(path).await.map_err(|e| {
            Error::Fs(FsError::Read {
                path: path.to_path_buf(),
                source: e.into(),
            })
        })
    }

    async fn read_dir(&self, path: impl AsRef<Path>) -> Res<fs::ReadDir> {
        let path = path.as_ref();
        fs::read_dir(path).await.map_err(|e| {
            Error::Fs(FsError::Read {
                path: path.to_path_buf(),
                source: e,
            })
        })
    }

    async fn remove_dir_all(&self, path: impl AsRef<Path>) -> Res {
        Ok(fs::remove_dir_all(path).await?)
    }

    async fn remove_file(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        fs::remove_file(path).await
    }

    async fn rename(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Res {
        Ok(fs::rename(from, to).await?)
    }

    async fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        self.write_byte_stream_via(path, temp_name(), body, None)
            .await
    }

    async fn write_staged_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        write(path.as_ref(), body, false, false).await
    }

    async fn write_byte_stream_via(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        temp_name: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
        tag: Option<&'static str>,
    ) -> Res {
        write_via(path.as_ref(), temp_name.as_ref(), body, false, true, tag).await
    }

    async fn sync_dir(&self, path: impl AsRef<Path> + Send) {
        #[cfg(unix)]
        sync_dir(path.as_ref()).await;
        #[cfg(not(unix))]
        let _ = path;
    }

    async fn sync_file(&self, path: impl AsRef<Path> + Send) -> Res {
        // Windows flushes only a file opened for writing; Unix syncs any.
        sync_file(path.as_ref(), cfg!(windows)).await
    }

    async fn create_private_dir(&self, path: impl AsRef<Path> + Send) -> Res {
        create_private_dir(path.as_ref()).await
    }

    async fn write_private_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        write(path.as_ref(), body, true, true).await
    }

    async fn lock_exclusive(&self, path: impl AsRef<Path> + Send) -> Res<super::LockGuard> {
        super::lock::lock_exclusive(path.as_ref()).await
    }

    async fn try_lock_exclusive(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> Res<Option<super::LockGuard>> {
        super::lock::try_lock_exclusive(path.as_ref()).await
    }
}

/// Flushes the file at `path` to its drive, through a handle opened for
/// writing where `write_first`, as Windows needs, else for reading.
///
/// The flush is best effort for a file that can't be opened for writing,
/// such as one marked read-only: it is flushed through a handle opened for
/// reading, or, where that fails too, left with a warning. A file that can't
/// be opened at all is an error.
async fn sync_file(path: &Path, write_first: bool) -> Res {
    let failed = |source| {
        Error::Fs(FsError::Write {
            path: path.to_path_buf(),
            source,
        })
    };
    if write_first {
        match fs::OpenOptions::new().write(true).open(path).await {
            Ok(file) => return file.sync_all().await.map_err(failed),
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                debug!(
                    "{} can't be opened for writing, so it is flushed through a read handle: {err}",
                    path.display()
                );
            }
            Err(err) => return Err(failed(err)),
        }
    }
    let file = fs::File::open(path).await.map_err(failed)?;
    if let Err(err) = file.sync_all().await {
        if !write_first {
            return Err(failed(err));
        }
        warn!("{} was not flushed to its drive: {err}", path.display());
    }
    Ok(())
}

impl Default for LocalStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalStorage {
    #[must_use]
    pub fn new() -> Self {
        LocalStorage {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use test_log::test;

    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;

    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;

    use crate::io::storage::LockGuard;
    use crate::io::storage::StorageExt;

    /// A read-only file, as a copy of one can be, can't be opened for
    /// writing: the flush Windows needs a write handle for falls back to a
    /// read handle, and the file is left as it was (25k follow-up).
    #[test(tokio::test)]
    async fn a_read_only_file_is_flushed_on_a_best_effort_basis() -> Res {
        let temp_dir = tempdir()?;
        let path = temp_dir.path().join("read-only");
        std::fs::write(&path, b"kept")?;
        let writable = std::fs::metadata(&path)?.permissions();
        let mut read_only = writable.clone();
        read_only.set_readonly(true);
        std::fs::set_permissions(&path, read_only)?;
        // Root opens any file for writing, so there is no refusal to test.
        let refused = std::fs::OpenOptions::new().write(true).open(&path).is_err();

        let synced = sync_file(&path, true).await;

        std::fs::set_permissions(&path, writable)?;
        if !refused {
            return Ok(());
        }
        assert!(synced.is_ok(), "{synced:?}");
        assert_eq!(std::fs::read(&path)?, b"kept");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_missing_file_is_not_flushed() {
        let temp_dir = tempdir().expect("a temporary folder");
        let path = temp_dir.path().join("missing");
        for write_first in [true, false] {
            assert!(sync_file(&path, write_first).await.is_err());
        }
    }

    #[test(tokio::test)]
    #[ignore = "doesn't work in CI: in CI the file has a `now` modification date"]
    async fn test_getting_file_modified_ts() -> Res {
        let storage = LocalStorage::default();
        let timestamp = storage.modified_timestamp(Path::new("")).await?;
        assert_eq!(
            timestamp.to_string(),
            "2024-01-15 11:31:00.615186989 UTC".to_string()
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_copy() -> Res {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("foo");

        let storage = LocalStorage::default();

        assert!(fs::metadata(&dest).await.is_err());
        storage
            .write_byte_stream(&dest, ByteStream::from_static(b"anything"))
            .await?;
        assert!(fs::metadata(dest).await.is_ok());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_dirs() -> Res {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("foo").join("bar");

        let storage = LocalStorage::default();

        assert!(fs::metadata(&dest).await.is_err());
        storage.create_dir_all(&dest).await?;
        assert!(fs::metadata(&dest).await.is_ok());
        storage.remove_dir_all(dest.parent().unwrap()).await?;
        assert!(fs::metadata(&dest).await.is_err());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_files() -> Res {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("foo");

        let storage = LocalStorage::default();

        {
            let mut new_file = storage.create_file(&dest).await?;
            new_file.write_all(b"Hello").await?;
            new_file.flush().await?;
        }

        let contents = fs::read(dest).await?;
        assert_eq!(contents, b"Hello");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_read_byte_stream() -> Res {
        let storage = LocalStorage::default();
        let stream = storage
            .read_byte_stream("fixtures/user-settings.mkfg")
            .await?;
        let bytes = stream.collect().await?.to_vec();

        // Verify we can read the known test file
        assert_ne!(bytes, [] as [u8; 0]);
        assert_eq!(bytes, fs::read("fixtures/user-settings.mkfg").await?);

        Ok(())
    }

    /// [`LocalStorage`] that, once, writes `replacement` over a file the moment
    /// a read of it starts, the way another process's lineage write lands:
    /// through a temp file renamed onto it.
    struct ReplaceOnRead {
        inner: LocalStorage,
        replacement: std::sync::Mutex<Option<Vec<u8>>>,
    }

    impl ReplaceOnRead {
        async fn replace(&self, path: &Path) -> Res {
            let replacement = self.replacement.lock().unwrap().take();
            if let Some(bytes) = replacement {
                self.inner
                    .write_byte_stream(path, ByteStream::from(bytes))
                    .await?;
            }
            Ok(())
        }
    }

    impl Storage for ReplaceOnRead {
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
        async fn create_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
            self.inner.create_file(path).await
        }
        async fn exists(&self, path: impl AsRef<Path>) -> bool {
            self.inner.exists(path).await
        }
        async fn is_file(&self, path: impl AsRef<Path>) -> bool {
            self.inner.is_file(path).await
        }
        async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<Timestamp> {
            self.inner.modified_timestamp(path).await
        }
        async fn open_file(&self, path: impl AsRef<Path> + Send) -> Res<fs::File> {
            let file = self.inner.open_file(path.as_ref()).await?;
            self.replace(path.as_ref()).await?;
            Ok(file)
        }
        async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
            let stream = self.inner.read_byte_stream(path.as_ref()).await?;
            self.replace(path.as_ref()).await?;
            Ok(stream)
        }
        async fn read_dir(&self, path: impl AsRef<Path> + Send + Sync) -> Res<fs::ReadDir> {
            self.inner.read_dir(path).await
        }
        async fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.remove_dir_all(path).await
        }
        async fn remove_file(&self, path: impl AsRef<Path> + Send) -> Result<(), std::io::Error> {
            self.inner.remove_file(path).await
        }
        async fn rename(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res {
            self.inner.rename(from, to).await
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
        async fn lock_exclusive(&self, path: impl AsRef<Path> + Send) -> Res<LockGuard> {
            self.inner.lock_exclusive(path).await
        }
        async fn try_lock_exclusive(
            &self,
            path: impl AsRef<Path> + Send,
        ) -> Res<Option<LockGuard>> {
            self.inner.try_lock_exclusive(path).await
        }
    }

    #[test(tokio::test)]
    async fn a_file_replaced_during_a_read_is_read_whole() -> Res {
        let temp_dir = tempdir()?;
        let path = temp_dir.path().join("data.json");
        let before = br#"["before"]"#.to_vec();
        for after in [br#"["a longer replacement"]"#.to_vec(), b"[]".to_vec()] {
            fs::write(&path, &before).await?;
            let storage = ReplaceOnRead {
                inner: LocalStorage::new(),
                replacement: std::sync::Mutex::new(Some(after.clone())),
            };

            let read = storage.read_bytes(&path).await?;

            assert_eq!(fs::read(&path).await?, after, "the file was replaced");
            assert!(
                read == before || read == after,
                "read {}",
                String::from_utf8_lossy(&read)
            );
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_write_no_parent() -> Res {
        let storage = LocalStorage::default();
        let result = storage
            .write_byte_stream("", ByteStream::from_static(b"test"))
            .await;
        assert!(result.is_err());
        Ok(())
    }

    #[test(tokio::test)]
    #[cfg(unix)]
    async fn test_write_permission_denied() -> Res {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir()?;
        let readonly_dir = temp_dir.path().join("readonly");
        let test_file = readonly_dir.join("test.txt");

        // Create directory and make it read-only
        fs::create_dir_all(&readonly_dir).await?;
        let mut perms = fs::metadata(&readonly_dir).await?.permissions();
        perms.set_mode(0o444); // Read-only for owner, group, and others
        fs::set_permissions(&readonly_dir, perms).await?;

        let storage = LocalStorage::default();
        let result = storage
            .write_byte_stream(&test_file, ByteStream::from_static(b"test"))
            .await;

        // Should fail with permission denied
        assert!(result.is_err());
        let error = result.unwrap_err();

        assert!(matches!(error, Error::Fs(FsError::Write { .. })));
        let error_msg = error.to_string();
        assert!(error_msg.contains("Permission denied"));

        // Restore permissions for cleanup
        let mut perms = fs::metadata(&readonly_dir).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&readonly_dir, perms).await?;

        Ok(())
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// A body of two chunks that, before it yields the second, records the
    /// mode of the temp file the first went into.
    #[cfg(unix)]
    struct TempModeProbe {
        dir: PathBuf,
        frames: u8,
        seen: std::sync::Arc<std::sync::Mutex<Option<u32>>>,
    }

    #[cfg(unix)]
    impl http_body::Body for TempModeProbe {
        type Data = bytes::Bytes;
        type Error = std::io::Error;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            self.frames += 1;
            if self.frames == 2 {
                let tmp = std::fs::read_dir(&self.dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| path.to_string_lossy().contains(".tmp-"))
                    .expect("the write goes to a temp file first");
                *self.seen.lock().unwrap() = Some(mode(&tmp));
            }
            std::task::Poll::Ready(
                (self.frames <= 2)
                    .then(|| Ok(http_body::Frame::data(bytes::Bytes::from_static(b"secret")))),
            )
        }
    }

    /// A private write replaces a world-readable file with one only its owner
    /// can read, and the temp file it renames into place is never readable by
    /// anyone else either.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn a_private_write_is_owner_only_from_its_temp_file_on() -> Res {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("tokens.json");
        std::fs::write(&dest, b"old")?;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644))?;

        let seen = std::sync::Arc::default();
        let body = TempModeProbe {
            dir: temp_dir.path().to_path_buf(),
            frames: 0,
            seen: std::sync::Arc::clone(&seen),
        };
        LocalStorage::default()
            .write_private_byte_stream(
                &dest,
                ByteStream::new(aws_smithy_types::body::SdkBody::from_body_1_x(body)),
            )
            .await?;

        let temp_mode = seen.lock().unwrap().expect("the probe ran");
        assert_eq!(temp_mode & 0o077, 0, "temp file mode {temp_mode:o}");
        assert_eq!(mode(&dest), 0o600);
        assert_eq!(fs::read(&dest).await?, b"secretsecret");
        Ok(())
    }

    /// A file where a private directory belongs is an error, and is left as it
    /// was.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn a_private_dir_over_a_file_fails_and_leaves_the_file() -> Res {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir()?;
        let file = temp_dir.path().join(".auth");
        std::fs::write(&file, b"")?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;

        let result = LocalStorage::default().create_private_dir(&file).await;

        assert!(matches!(
            result,
            Err(Error::Fs(FsError::DirectoryCreate { .. }))
        ));
        assert_eq!(mode(&file), 0o644);
        Ok(())
    }

    /// A private directory is owner-only, whether it is new or was made
    /// world-readable before.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn a_private_dir_is_owner_only_even_if_it_existed() -> Res {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir()?;
        let existing = temp_dir.path().join(".auth");
        std::fs::create_dir(&existing)?;
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755))?;
        let new = existing.join("quilt.test");

        let storage = LocalStorage::default();
        storage.create_private_dir(&existing).await?;
        storage.create_private_dir(&new).await?;

        assert_eq!(mode(&existing), 0o700);
        assert_eq!(mode(&new), 0o700);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_write_overwrites_existing_file() -> Res {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("data.txt");
        let storage = LocalStorage::default();

        storage
            .write_byte_stream(&dest, ByteStream::from_static(b"original"))
            .await?;
        storage
            .write_byte_stream(&dest, ByteStream::from_static(b"updated"))
            .await?;

        assert_eq!(fs::read(&dest).await?, b"updated");
        Ok(())
    }

    /// Records the level of each event logged while it is the default subscriber.
    #[cfg(unix)]
    #[derive(Clone, Default)]
    pub(crate) struct Logged(std::sync::Arc<std::sync::Mutex<Vec<tracing::Level>>>);

    #[cfg(unix)]
    impl Logged {
        /// Awaits `body`, returning its output and the levels it logged.
        pub(crate) async fn during<T>(body: impl Future<Output = T>) -> (T, Vec<tracing::Level>) {
            let logged = Logged::default();
            let guard = tracing::subscriber::set_default(logged.clone());
            let output = body.await;
            drop(guard);
            let levels = logged.0.lock().unwrap().clone();
            (output, levels)
        }
    }

    #[cfg(unix)]
    impl tracing::Subscriber for Logged {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            self.0.lock().unwrap().push(*event.metadata().level());
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// A folder its owner can write and search but not read, so it cannot be
    /// opened to sync it.
    #[cfg(unix)]
    struct UnreadableFolder {
        path: PathBuf,
        _temp_dir: tempfile::TempDir,
    }

    #[cfg(unix)]
    impl UnreadableFolder {
        /// `None` as root, which ignores the mode: there the sync would not fail.
        fn new() -> Res<Option<Self>> {
            use std::os::unix::fs::PermissionsExt;

            let temp_dir = tempdir()?;
            let path = temp_dir.path().join("unreadable");
            std::fs::create_dir(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o300))?;
            let folder = UnreadableFolder {
                path,
                _temp_dir: temp_dir,
            };
            Ok(std::fs::File::open(&folder.path).is_err().then_some(folder))
        }
    }

    #[cfg(unix)]
    impl Drop for UnreadableFolder {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o700));
        }
    }

    /// A directory that cannot be opened to sync it still takes the write,
    /// since the new file is in place by then, and the failed sync is logged.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn a_write_whose_directory_cannot_be_synced_succeeds_and_warns() -> Res {
        let Some(folder) = UnreadableFolder::new()? else {
            return Ok(());
        };
        let dest = folder.path.join("data.json");

        let (result, logged) = Logged::during(
            LocalStorage::default().write_byte_stream(&dest, ByteStream::from_static(b"new")),
        )
        .await;

        result?;
        assert_eq!(fs::read(&dest).await?, b"new");
        assert_eq!(logged, [tracing::Level::WARN]);
        Ok(())
    }

    /// A staged file is renamed elsewhere right after, so its folder is not
    /// synced: the same unreadable folder logs nothing.
    #[test(tokio::test)]
    #[cfg(unix)]
    async fn a_staged_write_does_not_sync_its_folder() -> Res {
        let Some(folder) = UnreadableFolder::new()? else {
            return Ok(());
        };
        let dest = folder.path.join("staged");

        let (result, logged) = Logged::during(
            LocalStorage::default()
                .write_staged_byte_stream(&dest, ByteStream::from_static(b"new")),
        )
        .await;

        result?;
        assert_eq!(fs::read(&dest).await?, b"new");
        assert!(logged.is_empty(), "{logged:?}");
        Ok(())
    }

    #[test(tokio::test)]
    #[cfg(unix)]
    async fn a_folder_that_cannot_be_synced_warns() -> Res {
        let Some(folder) = UnreadableFolder::new()? else {
            return Ok(());
        };
        let ((), logged) = Logged::during(LocalStorage::default().sync_dir(&folder.path)).await;
        assert_eq!(logged, [tracing::Level::WARN]);
        Ok(())
    }

    /// The names and contents of the files in `dir`.
    fn files_in(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(&path).unwrap_or_default())
            })
            .collect()
    }

    /// A body that fails before yielding anything.
    struct FailingBody;

    impl http_body::Body for FailingBody {
        type Data = bytes::Bytes;
        type Error = std::io::Error;

        fn poll_frame(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            std::task::Poll::Ready(Some(Err(std::io::Error::other("cut off"))))
        }
    }

    /// A temp name that is a path is refused before anything is written.
    #[test(tokio::test)]
    async fn a_temp_name_that_leaves_the_folder_is_refused() -> Res {
        let temp_dir = tempdir()?;
        let folder = temp_dir.path().join("folder");
        let outside = temp_dir.path().join("outside.tmp");

        for temp_name in [outside, std::path::PathBuf::from("../outside.tmp")] {
            let result = LocalStorage::default()
                .write_byte_stream_via(
                    folder.join("data.json"),
                    &temp_name,
                    ByteStream::from_static(b"new"),
                    None,
                )
                .await;
            assert!(result.is_err(), "{} was taken", temp_name.display());
        }
        assert!(files_in(temp_dir.path()).is_empty());
        Ok(())
    }

    /// A temp named after its target is refused, and the target left whole.
    #[test(tokio::test)]
    async fn a_temp_named_after_its_target_is_refused() -> Res {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("data.json");
        std::fs::write(&dest, b"old")?;

        let result = LocalStorage::default()
            .write_byte_stream_via(
                &dest,
                "data.json",
                ByteStream::new(aws_smithy_types::body::SdkBody::from_body_1_x(FailingBody)),
                None,
            )
            .await;

        assert!(result.is_err());
        assert_eq!(std::fs::read(&dest).ok(), Some(b"old".to_vec()));
        Ok(())
    }

    /// A write whose body fails removes its named temp: only a killed write
    /// leaves one.
    #[test(tokio::test)]
    async fn a_write_whose_body_fails_removes_its_named_temp() -> Res {
        let temp_dir = tempdir()?;

        let result = LocalStorage::default()
            .write_byte_stream_via(
                temp_dir.path().join("data.json"),
                "data.json.tmp-1",
                ByteStream::new(aws_smithy_types::body::SdkBody::from_body_1_x(FailingBody)),
                None,
            )
            .await;

        assert!(result.is_err());
        assert!(files_in(temp_dir.path()).is_empty());
        Ok(())
    }

    /// A write killed at each of its failpoints, with nothing unwinding.
    #[cfg(unix)]
    mod killed {
        use super::*;

        use std::collections::BTreeMap;

        use test_log::test;

        const CHILD_DEST: &str = "QUILT_RS_TEST_CHILD_DEST";
        const KILLED_TEMP: &str = "data.json.tmp-killed";

        /// Not a test on its own: the child half of [`killed_at`]. It writes `new`
        /// through the failpoints tagged `test`.
        #[test]
        #[ignore = "run only as the child of killed_at"]
        fn child_writes_through_the_failpoints() -> Res {
            let Ok(dest) = std::env::var(CHILD_DEST) else {
                return Ok(());
            };
            tokio::runtime::Runtime::new()?.block_on(LocalStorage::default().write_byte_stream_via(
                dest,
                KILLED_TEMP,
                ByteStream::from_static(b"new"),
                Some("test"),
            ))
        }

        /// Has a child process write `new` over `data.json`, which holds `old`,
        /// `SIGKILL`s it once it reaches the failpoint `test.<step>`, and returns
        /// the files left in the folder.
        fn killed_at(step: &str) -> Res<BTreeMap<String, Vec<u8>>> {
            use std::time::Duration;
            use std::time::Instant;

            let dir = tempdir()?;
            let signals = tempdir()?;
            let dest = dir.path().join("data.json");
            std::fs::write(&dest, b"old")?;
            let name = format!("test.{step}");
            let mut child = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "io::storage::local::tests::killed::child_writes_through_the_failpoints",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD_DEST, &dest)
                .env(crate::failpoint::NAME_VAR, &name)
                .env(crate::failpoint::DIR_VAR, signals.path())
                .spawn()?;

            let reached = signals.path().join(format!("{name}.reached"));
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut exited = None;
            while !reached.exists() && exited.is_none() && Instant::now() < deadline {
                exited = child.try_wait()?;
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            child.wait()?;
            assert!(
                reached.exists(),
                "the child never reached {name}; it exited with {exited:?}"
            );
            Ok(files_in(dir.path()))
        }

        fn files(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
            entries
                .iter()
                .map(|(name, bytes)| ((*name).to_string(), bytes.to_vec()))
                .collect()
        }

        #[test]
        fn a_write_killed_before_its_temp_leaves_the_old_file_alone() -> Res {
            assert_eq!(killed_at("before-temp")?, files(&[("data.json", b"old")]));
            Ok(())
        }

        /// The temp is whole before the rename, and a kill leaves it under the
        /// name the caller gave it.
        #[test]
        fn a_write_killed_before_its_rename_leaves_the_old_file_and_the_whole_temp() -> Res {
            assert_eq!(
                killed_at("after-temp")?,
                files(&[("data.json", b"old"), (KILLED_TEMP, b"new")])
            );
            Ok(())
        }

        #[test]
        fn a_write_killed_after_its_rename_leaves_the_whole_new_file() -> Res {
            assert_eq!(killed_at("after-rename")?, files(&[("data.json", b"new")]));
            Ok(())
        }
    }

    /// devfs on macOS and procfs on Linux refuse every folder sync, as network
    /// and FUSE mounts do; that refusal is logged at debug, not as a warning.
    #[test(tokio::test)]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    async fn a_folder_sync_the_file_system_refuses_is_logged_at_debug() {
        let folder = if cfg!(target_os = "macos") {
            "/dev"
        } else {
            "/proc"
        };
        let ((), logged) = Logged::during(sync_dir(Path::new(folder))).await;
        assert_eq!(logged, [tracing::Level::DEBUG]);
    }

    #[test]
    fn a_bare_file_name_is_written_into_the_current_folder() {
        assert_eq!(folder_of(Path::new("data.json")), Path::new("."));
        assert_eq!(
            folder_of(Path::new(".quilt/data.json")),
            Path::new(".quilt")
        );
    }

    #[test(tokio::test)]
    async fn test_write_leaves_no_temp_files() -> Res {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("data.txt");
        let storage = LocalStorage::default();

        // Successful write
        storage
            .write_byte_stream(&dest, ByteStream::from_static(b"hello"))
            .await?;

        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())?
            .filter_map(std::result::Result::ok)
            .collect();
        assert_eq!(entries.len(), 1, "only the target file should exist");
        assert_eq!(entries[0].file_name(), "data.txt");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_failed_write_leaves_no_temp_files() -> Res {
        let temp_dir = tempdir()?;
        let storage = LocalStorage::default();

        // Target is a directory — rename will fail with EISDIR
        let target = temp_dir.path().join("a_dir");
        fs::create_dir(&target).await?;
        let _ = storage
            .write_byte_stream(&target, ByteStream::from_static(b"data"))
            .await;

        let has_tmp = std::fs::read_dir(temp_dir.path())?
            .filter_map(std::result::Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with(".tmp-"));
        assert!(!has_tmp, "temp file should be cleaned up");

        Ok(())
    }
}
