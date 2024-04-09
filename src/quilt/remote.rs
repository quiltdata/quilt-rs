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

    async fn exists(&self, s3_uri: &S3Uri) -> Result<bool, Error>;

    async fn put_object(
        &mut self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Result<(), Error>;

    async fn put_object_and_checksum(
        &mut self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error>;
}
