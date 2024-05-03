use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use multihash::Multihash;
use tokio::io::AsyncRead;

use crate::uri::S3Uri;
use crate::Error;

pub mod s3;
pub mod utils; // TODO: make it private after refactoring package_s3_folder

#[cfg(test)]
pub mod mocks;

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
}
