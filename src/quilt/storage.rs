use std::path::Path;
use std::path::PathBuf;

use crate::Error;

pub mod fs;
pub mod s3;

// Mock storage is only available during testing.
#[cfg(test)]
pub mod mock_storage;

/// Storage operations for the underlying filesystem.
///
/// This trait encapsulates the filesystem operations that Quilt needs to perform.
#[allow(async_fn_in_trait)]
pub trait Storage {
    /// Check if a path exists in the filesystem.
    async fn exists(&self, path: impl AsRef<Path>) -> bool;

    /// Copy a file from one location to another.
    async fn copy(
        &self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> Result<u64, std::io::Error>;

    /// Recursively creates a directory and all of its parent components if they
    /// are missing.
    async fn create_dir_all(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error>;

    /// Removes a directory at this path, after removing all its contents.
    async fn remove_dir_all(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error>;

    /// Get the timestamp of the last modification of a file.
    async fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<chrono::DateTime<chrono::Utc>, Error>;

    /// Remove a file from the filesystem.
    async fn remove_file(&mut self, path: PathBuf) -> Result<(), std::io::Error> {
        tokio::fs::remove_file(path).await
    }

    /// Writes bytes to a file
    async fn write(&mut self, path: PathBuf, bytes: &[u8]) -> Result<(), Error>;
}
