use std::future::Future;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;

use crate::Error;

pub mod fs;
pub mod s3;

// Mock storage is only available during testing.
#[cfg(test)]
pub mod mock_storage;

/// Storage operations for the underlying filesystem.
///
/// This trait encapsulates the filesystem operations that Quilt needs to perform.
pub trait Storage {
    /// Check if a path exists in the filesystem.
    fn exists(&self, path: impl AsRef<Path>) -> impl Future<Output = bool>;

    /// Copy a file from one location to another.
    fn copy(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<u64, Error>> + Send;

    /// Recursively creates a directory and all of its parent components if they
    /// are missing.
    fn create_dir_all(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Removes a directory at this path, after removing all its contents.
    fn remove_dir_all(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Get the timestamp of the last modification of a file.
    fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> impl Future<Output = Result<DateTime<Utc>, Error>>;

    /// Remove a file from the filesystem.
    fn remove_file(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Writes bytes to a file
    fn write_file(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        bytes: &[u8],
    ) -> impl Future<Output = Result<(), Error>> + Send + Sync;

    fn open_file(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<tokio::fs::File, Error>> + Send;

    fn create_file(
        &self,
        path: impl AsRef<Path>,
    ) -> impl Future<Output = Result<tokio::fs::File, Error>>;

    fn read_file(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> + Send + Sync;

    fn read_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Result<ByteStream, Error>> + Send + Sync;
}
