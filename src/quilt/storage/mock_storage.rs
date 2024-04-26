use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use tempfile;

use crate::Error;

use super::Storage;

/// A mock implementation of the `Storage` trait.
pub(crate) struct MockStorage {
    pub(crate) temp_dir: tempfile::TempDir,
}

impl Default for MockStorage {
    fn default() -> Self {
        MockStorage {
            temp_dir: tempfile::tempdir().expect("Failed to create temporrary directory"),
        }
    }
}

impl MockStorage {}

pub fn relative_to_temp_dir(
    temp_dir: &impl AsRef<Path>,
    path: impl AsRef<Path>,
) -> impl AsRef<Path> {
    if path.as_ref().starts_with("/") {
        if path.as_ref().starts_with(temp_dir) {
            // already relative, for example, in recursive calls like `read_dir`
            temp_dir
                .as_ref()
                .join(path.as_ref().strip_prefix(temp_dir.as_ref()).unwrap())
        } else {
            temp_dir
                .as_ref()
                .join(path.as_ref().strip_prefix("/").unwrap())
        }
    } else {
        temp_dir.as_ref().join(path)
    }
}

async fn create_parent(path: impl AsRef<Path>) -> Result<(), Error> {
    Ok(fs::create_dir_all(path.as_ref().parent().unwrap()).await?)
}

impl Storage for MockStorage {
    async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, Error> {
        let from_path = relative_to_temp_dir(&self.temp_dir, &from);
        let to_path = relative_to_temp_dir(&self.temp_dir, &to);
        create_parent(&to_path).await?;
        Ok(fs::copy(from_path, to_path).await?)
    }

    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(fs::create_dir_all(rel_path).await?)
    }

    async fn remove_dir_all(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(fs::remove_dir_all(rel_path).await?)
    }

    /// Overwrite the `remove_file` method to do nothing.
    async fn remove_file(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        fs::remove_file(rel_path).await
    }

    /// Overwrite the `exists` method to check if the path is in the set of paths.
    async fn exists(&self, path: impl AsRef<std::path::Path>) -> bool {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        fs::metadata(rel_path).await.is_ok()
    }

    /// Return the current time as the modified timestamp.
    async fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<chrono::DateTime<chrono::Utc>, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        create_parent(&rel_path).await?;
        let modified = fs::metadata(rel_path).await.map(|m| m.modified())??;
        Ok(DateTime::<Utc>::from(modified))
    }

    /// Overwrite the `write` method
    async fn write_file(&self, path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        create_parent(&rel_path).await?;
        Ok(fs::write(rel_path, bytes).await?)
    }

    async fn open_file(&self, path: impl AsRef<Path>) -> Result<fs::File, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(fs::File::open(rel_path).await?)
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Result<fs::File, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        create_parent(&rel_path).await?;
        Ok(fs::File::create(rel_path).await?)
    }

    async fn read_dir(&self, path: impl AsRef<Path>) -> Result<fs::ReadDir, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(fs::read_dir(&rel_path).await?)
    }

    async fn read_file(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(fs::read(&rel_path).await?)
    }

    async fn read_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> Result<ByteStream, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(ByteStream::from_path(rel_path).await?)
    }

    async fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        mut body: ByteStream,
    ) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        let mut file = fs::File::create(&rel_path).await?;
        while let Some(bytes) = body.try_next().await? {
            file.write_all(&bytes).await?;
        }
        file.flush().await?;

        Ok(())
    }
}
