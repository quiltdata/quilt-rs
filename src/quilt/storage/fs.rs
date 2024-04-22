use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::Error;

use super::Storage;

async fn write(path: impl AsRef<Path> + Send, bytes: &[u8]) -> Result<(), Error> {
    let Some(parent) = path.as_ref().parent() else {
        return Err(Error::MissingParentPath(path.as_ref().to_owned()));
    };
    fs::create_dir_all(&parent).await?;

    // TODO: Write to a temporary location, then move.
    let mut file = fs::File::create(&path).await?;

    file.write_all(bytes).await?;

    Ok(())
}

pub async fn get_file_modified_ts(path: impl AsRef<Path>) -> Result<DateTime<Utc>, Error> {
    let modified = fs::metadata(path).await.map(|m| m.modified())??;
    Ok(DateTime::<Utc>::from(modified))
}

#[derive(Clone, Debug)]
pub struct LocalStorage {}

impl Storage for LocalStorage {
    async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, Error> {
        Ok(fs::copy(from, to).await?)
    }

    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        Ok(fs::create_dir_all(path).await?)
    }

    async fn remove_dir_all(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        Ok(fs::remove_dir_all(path).await?)
    }

    async fn remove_file(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        fs::remove_file(path).await
    }

    /// Check if a path exists in the filesystem.
    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        fs::metadata(path).await.is_ok()
    }

    async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Result<DateTime<Utc>, Error> {
        get_file_modified_ts(path).await
    }

    async fn write_file(&self, path: impl AsRef<Path> + Send, bytes: &[u8]) -> Result<(), Error> {
        write(path, bytes).await
    }

    async fn open_file(&self, path: impl AsRef<Path>) -> Result<fs::File, Error> {
        Ok(fs::File::open(path).await?)
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Result<fs::File, Error> {
        Ok(fs::File::create(path.as_ref()).await?)
    }

    async fn read_file(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, Error> {
        Ok(fs::read(&path).await?)
    }

    async fn read_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> Result<ByteStream, Error> {
        Ok(ByteStream::from_path(path).await?)
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

    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;

    use crate::utils::local_uri_json;

    #[tokio::test]
    #[ignore] // It doesn't work in CI. In CI file has `now` date
    async fn test_getting_file_modified_ts() -> Result<(), Error> {
        let timestamp = get_file_modified_ts(local_uri_json()).await?;
        assert_eq!(
            timestamp.to_string(),
            "2024-01-15 11:31:00.615186989 UTC".to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_copy() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("foo");

        let storage = LocalStorage::default();

        assert!(fs::metadata(&dest).await.is_err());
        storage.copy(local_uri_json(), &dest).await?;
        assert!(fs::metadata(dest).await.is_ok());

        Ok(())
    }

    #[tokio::test]
    async fn test_dirs() -> Result<(), Error> {
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

    #[tokio::test]
    async fn test_files() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let dest = temp_dir.path().join("foo");

        let storage = LocalStorage::default();

        let mut new_file = storage.create_file(&dest).await?;
        new_file.write_all(b"Hello").await?;
        let contents = fs::read(dest).await?;
        assert_eq!(contents, b"Hello");

        Ok(())
    }
}
