//!
//! Wraps operations with local storage. Mostly wraps tokio::fs.
//! It uses trait, so we can swap implementation for tests.

use std::future::Future;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use chrono::DateTime;
use chrono::Utc;
use tokio::fs::{File, ReadDir};

use crate::io::remote::S3Attributes;
use crate::uri::S3Uri;
use crate::Error;

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
    ) -> impl Future<Output = Result<u64, Error>> + Send;

    /// Recursively creates a directory and all of its parent components if they
    /// are missing.
    fn create_dir_all(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Creates file
    fn create_file(&self, path: impl AsRef<Path>) -> impl Future<Output = Result<File, Error>>;

    /// Check if a path exists in the filesystem.
    fn exists(&self, path: impl AsRef<Path>) -> impl Future<Output = bool>;

    /// Get the same attributes including checskum as from S3
    fn get_object_attributes(
        &self,
        listing_uri: &S3Uri,
        object_key: impl AsRef<str> + Send + Sync,
    ) -> impl Future<Output = Result<S3Attributes, Error>> + Send + Sync;

    /// Get the timestamp of the last modification of a file.
    fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> impl Future<Output = Result<DateTime<Utc>, Error>>;

    /// Opens file (doesn't read contents)
    fn open_file(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<File, Error>> + Send;

    /// Reads the entire contents of a file into a stream.
    fn read_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Result<ByteStream, Error>> + Send + Sync;

    /// Returns a stream over the entries within a directory.
    /// Not recursive.
    fn read_dir(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Result<ReadDir, Error>> + Send + Sync;

    /// Reads the entire contents of a file into a bytes vector.
    /// Prefer using `read_byte_stream`.
    // TODO: Remove it in favor of `self.read_byte_stream`
    fn read_file(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> + Send + Sync;

    /// Removes a directory at this path, after removing all its contents.
    fn remove_dir_all(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = Result<(), Error>> + Send;

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
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Writes bytes srteam to a file
    fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> impl Future<Output = Result<(), Error>> + Send + Sync;

    /// Writes bytes to a file
    /// Prefer using `write_byte_stream`.
    // TODO: Remove it in favor of `self.write_byte_stream`
    fn write_file(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        bytes: &[u8],
    ) -> impl Future<Output = Result<(), Error>> + Send + Sync;
}
