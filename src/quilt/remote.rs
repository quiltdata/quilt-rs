use tokio::io::AsyncRead;

use crate::Error;

pub mod mock_remote;

/// This trait encapsulates the S3 operations that Quilt needs to perform.
#[allow(async_fn_in_trait)]
pub trait Remote {
    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<impl AsyncRead + Send + Unpin, Error>;

    async fn exists(&self, bucket: &str, key: &str) -> Result<bool, Error>;
}
