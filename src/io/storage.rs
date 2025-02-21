//!
//! Wraps operations with local storage. Mostly wraps tokio::fs.
//! It uses trait, so we can swap implementation for tests.

use std::future::Future;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::Object;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs::File;
use tokio::fs::ReadDir;

use crate::io::remote::RemoteObjectStream;
use crate::io::remote::S3Attributes;
use crate::uri::S3Uri;
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

    /// Get the same attributes including checskum as from S3
    fn get_object_attributes(
        &self,
        stream: RemoteObjectStream,
        listing_uri: &S3Uri,
        object: &Object,
    ) -> impl Future<Output = Res<S3Attributes>> + Send + Sync;

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

    /// Reads the entire contents of a file into a bytes vector.
    /// Prefer using `read_byte_stream`.
    // TODO: Remove it in favor of `self.read_byte_stream`
    fn read_file(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Res<Vec<u8>>> + Send + Sync;

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

    /// Writes bytes srteam to a file
    fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> impl Future<Output = Res> + Send + Sync;

    /// Writes bytes to a file
    /// Prefer using `write_byte_stream`.
    // TODO: Remove it in favor of `self.write_byte_stream`
    fn write_file(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        bytes: &[u8],
    ) -> impl Future<Output = Res> + Send + Sync;
}
