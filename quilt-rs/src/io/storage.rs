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
pub(crate) mod lock;

pub use local::LocalStorage;
pub use lock::LockGuard;

// Mock storage is available during testing, or to downstream crates via the
// `testing` feature.
#[cfg(any(test, feature = "testing"))]
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

    /// Check if a path is a regular file, following links.
    fn is_file(&self, path: impl AsRef<Path>) -> impl Future<Output = bool>;

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

    /// Creates the directory `path` and any missing parent. On Unix `path` is
    /// created 0700, and one that already existed is set to 0700 where the
    /// file system allows. On Windows it is [`Self::create_dir_all`].
    fn create_private_dir(&self, path: impl AsRef<Path> + Send)
    -> impl Future<Output = Res> + Send;

    /// [`Self::write_byte_stream`] for a file only its owner may read: on Unix
    /// the file is 0600 from its creation, before any byte is written or it is
    /// renamed into place. On Windows it is [`Self::write_byte_stream`].
    fn write_private_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> impl Future<Output = Res> + Send + Sync;

    /// Locks `path` exclusively until the guard drops: against every other
    /// holder in this process, through any handle on the same file, and in any
    /// other process. Creates the file if it is missing, 0600 on Unix.
    ///
    /// A holder that awaits the network keeps every other holder waiting for
    /// as long.
    fn lock_exclusive(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Res<LockGuard>> + Send;

    /// [`Self::lock_exclusive`] without the wait: `None` while another holder
    /// has the lock.
    fn try_lock_exclusive(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Res<Option<LockGuard>>> + Send;
}

impl<S: Storage + Send + Sync> Storage for Arc<S> {
    async fn copy(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res<u64> {
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

    async fn is_file(&self, path: impl AsRef<Path>) -> bool {
        (**self).is_file(path).await
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

    async fn rename(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res {
        (**self).rename(from, to).await
    }

    async fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        (**self).write_byte_stream(path, body).await
    }

    async fn create_private_dir(&self, path: impl AsRef<Path> + Send) -> Res {
        (**self).create_private_dir(path).await
    }

    async fn write_private_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        (**self).write_private_byte_stream(path, body).await
    }

    async fn lock_exclusive(&self, path: impl AsRef<Path> + Send) -> Res<LockGuard> {
        (**self).lock_exclusive(path).await
    }

    async fn try_lock_exclusive(&self, path: impl AsRef<Path> + Send) -> Res<Option<LockGuard>> {
        (**self).try_lock_exclusive(path).await
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
