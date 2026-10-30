use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
#[cfg(unix)]
use tracing::warn;
use uuid::Uuid;

use crate::Error;
use crate::Res;
use crate::error::FsError;

use super::Storage;

/// Generate a unique file path in `dir` for use as a temp file.
///
/// Same directory as the target ensures `rename` is atomic (same filesystem).
fn temp_path_in(dir: &Path) -> PathBuf {
    dir.join(format!(".tmp-{}", Uuid::new_v4()))
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

/// Write `body` to a temp file, then atomically rename to `path`.
///
/// On failure the temp file is cleaned up.
async fn atomic_write(path: &Path, body: ByteStream, private: bool) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).await?;
    let tmp = temp_path_in(parent);
    let cleanup = |_: &std::io::Error| {
        let _ = std::fs::remove_file(&tmp);
    };
    write_stream(&tmp, body, private)
        .await
        .inspect_err(cleanup)?;
    fs::rename(&tmp, path).await.inspect_err(cleanup)
}

pub(super) async fn write(path: &Path, body: ByteStream, private: bool) -> Res {
    atomic_write(path, body, private).await.map_err(|source| {
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

    async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<DateTime<Utc>> {
        let modified = fs::metadata(path).await.map(|m| m.modified())??;
        Ok(DateTime::<Utc>::from(modified))
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
        write(path.as_ref(), body, false).await
    }

    async fn create_private_dir(&self, path: impl AsRef<Path> + Send) -> Res {
        create_private_dir(path.as_ref()).await
    }

    async fn write_private_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        write(path.as_ref(), body, true).await
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
mod tests {
    use super::*;

    use test_log::test;

    use std::path::Path;

    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;

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
