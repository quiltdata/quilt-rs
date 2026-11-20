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
use tokio::io::AsyncReadExt;

use crate::Res;
use crate::error::FsError;

pub mod auth;
mod local;
pub(crate) mod lock;

#[cfg(all(test, unix))]
pub(crate) use local::tests::Logged;

pub use local::LocalStorage;
pub(crate) use local::temp_name;
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

    /// Removes an empty directory, and fails, leaving it, if it holds anything:
    /// unlike [`remove_dir_all`](Self::remove_dir_all) it cannot delete a file
    /// that appeared in it a moment ago. A storage that does not override this
    /// removes nothing and fails.
    fn remove_dir(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = Res> + Send {
        let _ = path;
        async {
            Err(crate::Error::Io(std::io::Error::from(
                std::io::ErrorKind::Unsupported,
            )))
        }
    }

    /// Check if a path is itself a symbolic link, without following it. A
    /// storage that does not override this reports none.
    fn is_symlink(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = bool> + Send {
        let _ = path;
        async { false }
    }

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

    /// [`Self::write_byte_stream`] for a file renamed elsewhere right after it
    /// lands, such as a download staged until it is checked: the folder it
    /// lands in is not synced, since nothing needs it there after a power loss.
    fn write_staged_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> impl Future<Output = Res> + Send + Sync {
        self.write_byte_stream(path, body)
    }

    /// [`Self::write_byte_stream`] through a temp file named `temp_name` in
    /// `path`'s folder, so a leftover from a killed write can be traced to its
    /// target. The name is a bare file name, not the target's, and unique to
    /// this write: the write truncates a file already there, and removes it on
    /// failure. With a `tag`, the write passes the failpoints
    /// `<tag>.before-temp`, `<tag>.after-temp` and `<tag>.after-rename`. A
    /// storage that does not override this ignores both.
    fn write_byte_stream_via(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        temp_name: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
        tag: Option<&'static str>,
    ) -> impl Future<Output = Res> + Send + Sync {
        let _ = (temp_name, tag);
        self.write_byte_stream(path, body)
    }

    /// Makes the entries created, renamed or removed in the folder `path`
    /// survive a power loss. A failure is logged, not returned. On Windows,
    /// which has no folder sync, and in a storage that does not override this,
    /// it does nothing.
    fn sync_dir(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = ()> + Send {
        let _ = path;
        async {}
    }

    /// Makes the bytes of the file at `path`, written by something other than
    /// [`Self::write_byte_stream`] such as [`Self::copy`], survive a power
    /// loss. A storage that does not override this does nothing.
    fn sync_file(&self, path: impl AsRef<Path> + Send) -> impl Future<Output = Res> + Send {
        let _ = path;
        async { Ok(()) }
    }

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

    async fn remove_dir(&self, path: impl AsRef<Path> + Send) -> Res {
        (**self).remove_dir(path).await
    }

    async fn is_symlink(&self, path: impl AsRef<Path> + Send) -> bool {
        (**self).is_symlink(path).await
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

    async fn write_staged_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        (**self).write_staged_byte_stream(path, body).await
    }

    async fn write_byte_stream_via(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        temp_name: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
        tag: Option<&'static str>,
    ) -> Res {
        (**self)
            .write_byte_stream_via(path, temp_name, body, tag)
            .await
    }

    async fn sync_dir(&self, path: impl AsRef<Path> + Send) {
        (**self).sync_dir(path).await;
    }

    async fn sync_file(&self, path: impl AsRef<Path> + Send) -> Res {
        (**self).sync_file(path).await
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
    ) -> impl Future<Output = Res<Vec<u8>>> + Send;
}

impl<T: Storage + Sync> StorageExt for T {
    // Not through `read_byte_stream`: it takes the length before it opens the
    // file, so a file renamed over `path` in between reads cut to the old size.
    async fn read_bytes(&self, path: impl AsRef<Path> + Send + Sync) -> Res<Vec<u8>> {
        let path = path.as_ref();
        let mut file = self.open_file(path).await?;
        // Unsized, tokio reads in doubling chunks, each a blocking call.
        let size = file.metadata().await.map_or(0, |m| m.len());
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        file.read_to_end(&mut bytes)
            .await
            .map_err(|source| FsError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(bytes)
    }
}
