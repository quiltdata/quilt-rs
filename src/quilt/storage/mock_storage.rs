use std::path::Path;

use chrono::DateTime;
use chrono::Utc;

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

fn relative_to_temp_dir(temp_dir: &tempfile::TempDir, path: impl AsRef<Path>) -> impl AsRef<Path> {
    if path.as_ref().starts_with("/") {
        temp_dir
            .as_ref()
            .join(path.as_ref().strip_prefix("/").unwrap())
    } else {
        temp_dir.as_ref().join(&path)
    }
}

async fn create_parent(path: impl AsRef<Path>) -> Result<(), Error> {
    Ok(tokio::fs::create_dir_all(path.as_ref().parent().unwrap()).await?)
}

impl Storage for MockStorage {
    async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, Error> {
        let from_path = relative_to_temp_dir(&self.temp_dir, &from);
        let to_path = relative_to_temp_dir(&self.temp_dir, &to);
        create_parent(&to_path).await?;
        Ok(tokio::fs::copy(from_path, to_path).await?)
    }

    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(tokio::fs::create_dir_all(rel_path).await?)
    }

    async fn remove_dir_all(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(tokio::fs::remove_dir_all(rel_path).await?)
    }

    /// Overwrite the `remove_file` method to do nothing.
    async fn remove_file(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        tokio::fs::remove_file(rel_path).await
    }

    /// Overwrite the `exists` method to check if the path is in the set of paths.
    async fn exists(&self, path: impl AsRef<std::path::Path>) -> bool {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        tokio::fs::metadata(rel_path).await.is_ok()
    }

    /// Return the current time as the modified timestamp.
    async fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<chrono::DateTime<chrono::Utc>, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        create_parent(&rel_path).await?;
        let modified = tokio::fs::metadata(rel_path)
            .await
            .map(|m| m.modified())??;
        Ok(DateTime::<Utc>::from(modified))
    }

    /// Overwrite the `write` method to do nothing.
    async fn write_file(&self, path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        create_parent(&rel_path).await?;
        Ok(tokio::fs::write(rel_path, bytes).await?)
    }

    async fn open_file(&self, path: impl AsRef<Path>) -> Result<tokio::fs::File, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(tokio::fs::File::open(rel_path).await?)
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Result<tokio::fs::File, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        create_parent(&rel_path).await?;
        Ok(tokio::fs::File::create(rel_path).await?)
    }

    async fn read_file(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, Error> {
        let rel_path = relative_to_temp_dir(&self.temp_dir, &path);
        Ok(tokio::fs::read(&rel_path).await?)
    }
}
