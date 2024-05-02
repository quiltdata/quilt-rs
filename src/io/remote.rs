use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncRead;

use crate::uri::S3Uri;
use crate::Error;

pub mod s3;
pub mod utils; // TODO: make it private after refactoring package_s3_folder

#[cfg(test)]
pub mod mocks;

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
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error>;
}
