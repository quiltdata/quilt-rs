use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::Object;
use multihash::Multihash;
use tokio::io::AsyncRead;
use tokio_stream::Stream;

use crate::uri::S3Uri;
use crate::Error;

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

/// This trait encapsulates the S3 operations that Quilt needs to perform.
#[allow(async_fn_in_trait)]
pub trait Remote {
    async fn get_object(&self, s3_uri: &S3Uri) -> Result<impl AsyncRead + Send + Unpin, Error>;

    async fn get_object_stream(&self, s3_uri: &S3Uri) -> Result<ByteStream, Error>;

    async fn exists(&self, s3_uri: &S3Uri) -> Result<bool, Error>;

    async fn put_object(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Result<(), Error>;

    async fn upload_file(
        &self,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Result<(S3Uri, Multihash<256>), Error>;

    async fn get_object_attributes(
        &self,
        listing_uri: &S3Uri,
        object_key: impl AsRef<str>,
    ) -> Result<S3Attributes, Error>;

    // return Result<Row> as Item
    async fn list_objects(&self, listing_uri: S3Uri) -> impl Stream<Item = Result<Object, Error>>;
}
