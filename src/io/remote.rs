//!
//! Wraps operations with remote storage. Primarily S3.
//! It uses trait, so we can swap implementation for tests.

use std::future::Future;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::Object;
use tokio_stream::Stream;

use crate::checksum::ObjectHash;
use crate::uri::Host;
use crate::uri::S3Uri;
use crate::Res;

pub mod client;
mod host;
mod s3;
mod workflow;

pub use client::HttpClient;
pub use host::{fetch_host_config, HostChecksums, HostConfig};
pub use s3::RemoteS3;
pub use workflow::resolve_workflow;

#[cfg(test)]
pub mod mocks;

pub struct RemoteObjectStream {
    pub body: ByteStream,
    pub uri: S3Uri,
}

pub type StreamObjectChunk = Vec<Res<Object>>;

pub type StreamItem = Res<StreamObjectChunk>;

pub trait ObjectsStream: Stream<Item = StreamItem> {}

impl<T: Stream<Item = StreamItem>> ObjectsStream for T {}

/// This trait encapsulates the S3 operations that Quilt needs to perform.
pub trait Remote {
    /// Checks if object exists
    fn exists(&self, host: &Option<Host>, s3_uri: &S3Uri)
        -> impl Future<Output = Res<bool>> + Send;

    /// Fetches the objects contents as a `ByteStream`
    fn get_object_stream(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<RemoteObjectStream>> + Send;

    // Makes a head request and resolves the final versioned URL
    fn resolve_url(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<S3Uri>> + Send;

    /// Upload file. Just that
    fn put_object(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> impl Future<Output = Res>;

    /// Upload file and request checkum from S3
    fn upload_file(
        &self,
        host_config: &HostConfig,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> impl Future<Output = Res<(S3Uri, ObjectHash)>>;

    /// Fetch host configuration from the given host
    fn host_config(&self, host: &Option<Host>) -> impl Future<Output = Res<HostConfig>> + Send;
}
