use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::Error;
use crate::Res;

use super::Storage;

/// Create a temporary file in `dir` and return its path.
///
/// The temp file lives on the same filesystem as the target so that
/// a subsequent `rename` is always atomic.
fn temp_path_in(dir: &Path) -> std::io::Result<PathBuf> {
    // We only need the path — the fd is closed and we write via tokio.
    let (_, temp_path) = tempfile::NamedTempFile::new_in(dir)?.keep()?;
    Ok(temp_path)
}

/// Write `body` to a temp file, then atomically rename to `path`.
///
/// On failure the temp file is cleaned up.
async fn atomic_write(path: &Path, mut body: ByteStream) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).await?;
    let tmp = temp_path_in(parent)?;
    let result = async {
        let mut file = fs::File::create(&tmp).await?;
        while let Some(bytes) = body.try_next().await.map_err(std::io::Error::other)? {
            file.write_all(&bytes).await?;
        }
        file.flush().await?;
        fs::rename(&tmp, path).await
    }
    .await;
    if let Err(e) = result {
        // Prefer the original write/rename error over a cleanup failure.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    result
}

/// Implementation of the `Storage` trait for the local filesystem
#[derive(Clone, Debug)]
pub struct LocalStorage {}

impl Storage for LocalStorage {
    async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Res<u64> {
        let from = from.as_ref();
        let to = to.as_ref();
        fs::copy(from, to).await.map_err(|e| Error::FileCopy {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source: e,
        })
    }

    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Res {
        let path = path.as_ref();
        fs::create_dir_all(path)
            .await
            .map_err(|e| Error::DirectoryCreate {
                path: path.to_path_buf(),
                source: e,
            })
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
        let path = path.as_ref();
        fs::File::create(path).await.map_err(|e| Error::FileWrite {
            path: path.to_path_buf(),
            source: e,
        })
    }

    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        fs::metadata(path).await.is_ok()
    }

    async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<DateTime<Utc>> {
        let modified = fs::metadata(path).await.map(|m| m.modified())??;
        Ok(DateTime::<Utc>::from(modified))
    }

    async fn open_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
        let path = path.as_ref();
        fs::File::open(path).await.map_err(|e| Error::FileRead {
            path: path.to_path_buf(),
            source: e,
        })
    }

    async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
        let path = path.as_ref();
        ByteStream::from_path(path)
            .await
            .map_err(|e| Error::FileRead {
                path: path.to_path_buf(),
                source: e.into(),
            })
    }

    async fn read_dir(&self, path: impl AsRef<Path>) -> Res<fs::ReadDir> {
        let path = path.as_ref();
        fs::read_dir(path).await.map_err(|e| Error::FileRead {
            path: path.to_path_buf(),
            source: e,
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
        let path = path.as_ref();
        atomic_write(path, body).await.map_err(|source| Error::FileWrite {
            path: path.to_path_buf(),
            source,
        })
    }
}

impl Default for LocalStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalStorage {
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
    #[ignore] // It doesn't work in CI. In CI file has `now` date
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
        assert!(!bytes.is_empty());
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

        assert!(matches!(
            error,
            Error::DirectoryCreate { .. } | Error::FileWrite { .. }
        ));
        let error_msg = error.to_string();
        assert!(error_msg.contains("readonly") && error_msg.contains("Permission denied"));

        // Restore permissions for cleanup
        let mut perms = fs::metadata(&readonly_dir).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&readonly_dir, perms).await?;

        Ok(())
    }
}
