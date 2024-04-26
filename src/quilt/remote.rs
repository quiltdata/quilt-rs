use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncRead;

use crate::quilt::s3::S3Uri;
use crate::Error;

#[cfg(test)]
pub mod mock_remote;

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

    async fn put_object_and_checksum(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error>;

    async fn multipart_upload_and_checksum(
        &self,
        s3_uri: &S3Uri,
        file_path: impl AsRef<Path>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error>;
}
