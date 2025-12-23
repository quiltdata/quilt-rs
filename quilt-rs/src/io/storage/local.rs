use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::Error;
use crate::Res;

use super::Storage;

async fn write(path: impl AsRef<Path> + Send, bytes: &[u8]) -> Res {
    let Some(parent) = path.as_ref().parent() else {
        return Err(Error::MissingParentPath(path.as_ref().to_owned()));
    };
    fs::create_dir_all(&parent).await?;

    // TODO: Write to a temporary location, then move.
    let mut file = fs::File::create(&path).await?;

    file.write_all(bytes).await?;
    file.flush().await?;

    Ok(())
}

/// Implementation of the `Storage` trait for the local filesystem
#[derive(Clone, Debug)]
pub struct LocalStorage {}

impl Storage for LocalStorage {
    async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Res<u64> {
        Ok(fs::copy(from, to).await?)
    }

    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Res {
        Ok(fs::create_dir_all(path).await?)
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
        Ok(fs::File::create(path.as_ref()).await?)
    }

    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        fs::metadata(path).await.is_ok()
    }

    async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<DateTime<Utc>> {
        let modified = fs::metadata(path).await.map(|m| m.modified())??;
        Ok(DateTime::<Utc>::from(modified))
    }

    async fn open_file(&self, path: impl AsRef<Path>) -> Res<fs::File> {
        Ok(fs::File::open(path).await?)
    }

    async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
        Ok(ByteStream::from_path(path).await?)
    }

    async fn read_dir(&self, path: impl AsRef<Path>) -> Res<fs::ReadDir> {
        Ok(fs::read_dir(&path).await?)
    }

    async fn read_file(&self, path: impl AsRef<Path>) -> Res<Vec<u8>> {
        Ok(fs::read(&path).await?)
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
        mut body: ByteStream,
    ) -> Res {
        let mut file = fs::File::create(&path).await?;
        while let Some(bytes) = body.try_next().await? {
            file.write_all(&bytes).await?;
        }
        file.flush().await?;

        Ok(())
    }

    async fn write_file(&self, path: impl AsRef<Path> + Send, bytes: &[u8]) -> Res {
        write(path, bytes).await
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

    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;

    use crate::fixtures;

    #[test(tokio::test)]
    #[ignore] // It doesn't work in CI. In CI file has `now` date
    async fn test_getting_file_modified_ts() -> Res {
        let storage = LocalStorage::default();
        let timestamp = storage
            .modified_timestamp(fixtures::manifest::jsonl()?)
            .await?;
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
        storage.copy(fixtures::manifest::jsonl()?, &dest).await?;
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

        let mut new_file = storage.create_file(&dest).await?;
        new_file.write_all(b"Hello").await?;
        let contents = fs::read(dest).await?;
        assert_eq!(contents, b"Hello");

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_read_byte_stream() -> Res {
        let storage = LocalStorage::default();
        let stream = storage
            .read_byte_stream(fixtures::manifest::jsonl()?)
            .await?;
        let bytes = stream.collect().await?.to_vec();

        // Verify we can read the known test file
        assert!(!bytes.is_empty());
        assert_eq!(bytes, fs::read(fixtures::manifest::jsonl()?).await?);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_write_no_parent() -> Res {
        let storage = LocalStorage::default();
        let result = storage.write_file("", b"test").await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Missing parent path error: "
        );
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
        let result = storage.write_file(&test_file, b"test").await;

        // Should fail with permission denied
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(matches!(error, Error::Io(_)));

        // Restore permissions for cleanup
        let mut perms = fs::metadata(&readonly_dir).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&readonly_dir, perms).await?;

        Ok(())
    }
}
