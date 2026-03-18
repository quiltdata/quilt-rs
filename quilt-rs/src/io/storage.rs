//!
//! Wraps operations with local storage. Mostly wraps tokio::fs.
//! It uses trait, so we can swap implementation for tests.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

pub use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs::File;
use tokio::fs::ReadDir;

use crate::Res;

pub mod auth;
mod local;

pub use local::LocalStorage;

// Mock storage is only available during testing.
#[cfg(test)]
pub mod mocks;

/// Storage operations for the underlying filesystem.
///
/// This trait encapsulates the filesystem operations that Quilt needs to perform.
pub trait Storage {
    /// Copy a file from one location to another.
    fn copy(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Res<u64>> + Send;

    /// Recursively creates a directory and all of its parent components if they
    /// are missing.
    fn create_dir_all(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = Res> + Send;

    /// Creates file
    fn create_file(&self, path: impl AsRef<Path>) -> impl Future<Output = Res<File>>;

    /// Check if a path exists in the filesystem.
    fn exists(&self, path: impl AsRef<Path>) -> impl Future<Output = bool>;

    /// Get the timestamp of the last modification of a file.
    fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> impl Future<Output = Res<DateTime<Utc>>>;

    /// Opens file (doesn't read contents)
    fn open_file(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = Res<File>> + Send;

    /// Reads the entire contents of a file into a stream.
    fn read_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Res<ByteStream>> + Send + Sync;

    /// Returns a stream over the entries within a directory.
    /// Not recursive.
    fn read_dir(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Res<ReadDir>> + Send + Sync;

    /// Removes a directory at this path, after removing all its contents.
    fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = Res> + Send;

    /// Remove a file from the filesystem.
    fn remove_file(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Rename/move a file from one location to another.
    fn rename(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Res> + Send;

    /// Writes byte stream to a file
    fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> impl Future<Output = Res> + Send + Sync;
}

impl<S: Storage + Send + Sync> Storage for Arc<S> {
    async fn copy(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> Res<u64> {
        (**self).copy(from, to).await
    }

    async fn create_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
        (**self).create_dir_all(path).await
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Res<File> {
        (**self).create_file(path).await
    }

    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        (**self).exists(path).await
    }

    async fn modified_timestamp(&self, path: impl AsRef<Path>) -> Res<DateTime<Utc>> {
        (**self).modified_timestamp(path).await
    }

    async fn open_file(&self, path: impl AsRef<Path> + Send) -> Res<File> {
        (**self).open_file(path).await
    }

    async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
        (**self).read_byte_stream(path).await
    }

    async fn read_dir(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ReadDir> {
        (**self).read_dir(path).await
    }

    async fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
        (**self).remove_dir_all(path).await
    }

    async fn remove_file(&self, path: impl AsRef<Path> + Send) -> Result<(), std::io::Error> {
        (**self).remove_file(path).await
    }

    async fn rename(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> Res {
        (**self).rename(from, to).await
    }

    async fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        (**self).write_byte_stream(path, body).await
    }
}

/// Convenience methods on top of `Storage`.
///
/// Automatically implemented for all `Storage` types.
pub trait StorageExt: Storage {
    /// Read the entire contents of a file into a byte vector.
    fn read_bytes(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Res<Vec<u8>>> + Send + Sync;
}

impl<T: Storage + Sync> StorageExt for T {
    async fn read_bytes(&self, path: impl AsRef<Path> + Send + Sync) -> Res<Vec<u8>> {
        Ok(self.read_byte_stream(path).await?.collect().await?.to_vec())
    }
}
