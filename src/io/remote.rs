//!
//! Wraps operations with remote storage. Primarily S3.
//! It uses trait, so we can swap implementation for tests.

use std::future::Future;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::Object;
use multihash::Multihash;
use tokio_stream::Stream;

use crate::uri::S3Uri;
use crate::Res;

mod client;
mod s3;

pub use client::get_client_for_bucket;
pub use s3::RemoteS3;

#[cfg(test)]
pub mod mocks;

/// We use it for getting hashes in files listings when we create new packages from S3 directory.
/// Also, we re-use this struct for calculating hashes locally when S3-checksums are disabled.
pub struct S3Attributes {
    pub listing_uri: S3Uri,
    pub object_uri: S3Uri,
    pub hash: Multihash<256>,
    pub size: u64,
}

pub struct HeadObject {
    pub version: Option<String>,
    pub size: u64,
}

pub struct GetObject {
    pub head: HeadObject,
    pub stream: ByteStream,
}

pub type StreamObjectChunk = Vec<Res<Object>>;

pub type StreamItem = Res<StreamObjectChunk>;

pub trait ObjectsStream: Stream<Item = StreamItem> {}

impl<T: Stream<Item = StreamItem>> ObjectsStream for T {}

/// This trait encapsulates the S3 operations that Quilt needs to perform.
pub trait Remote {
    /// Checks if object exists
    fn exists(&self, s3_uri: &S3Uri) -> impl Future<Output = Res<bool>> + Send;

    fn head_object(&self, s3_uri: &S3Uri) -> impl Future<Output = Res<HeadObject>> + Send;

    /// Get object attributes: checksums, number of chunks, chunksize, version_id
    fn get_object_attributes(
        &self,
        listing_uri: &S3Uri,
        object_key: impl AsRef<str>,
    ) -> impl Future<Output = Res<S3Attributes>>;

    /// Fetches the objects contents as a `ByteStream`
    fn get_object_stream(&self, s3_uri: &S3Uri) -> impl Future<Output = Res<GetObject>> + Send;

    /// List objects list under S3 prefix using tokio Stream
    // TODO: return Item = Res<Row>
    fn list_objects(&self, listing_uri: S3Uri) -> impl Future<Output = impl ObjectsStream> + Send;

    /// Upload file. Just that
    fn put_object(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> impl Future<Output = Res>;

    /// Upload file and request checkum from S3
    fn upload_file(
        &self,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> impl Future<Output = Res<(S3Uri, Multihash<256>)>>;
}
