use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tracing::log;

use crate::checksum;
use crate::io::storage::mocks::MockStorage;
use crate::io::storage::Storage;
use crate::uri::S3Uri;
use crate::Error;

use super::Remote;

/// A mock implementation of the `Remote` trait.
#[derive(Default)]
pub(crate) struct MockRemote {
    pub(crate) storage: MockStorage,
}

impl Remote for MockRemote {
    async fn get_object(&self, s3_uri: &S3Uri) -> Result<impl AsyncRead + Send + Unpin, Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} get request", key);

        self.storage.open_file(&key).await.map_err(|err| match err {
            Error::Io(inner_err) => {
                if inner_err.kind() == std::io::ErrorKind::NotFound {
                    Error::S3("Key doesn't exists".to_string())
                } else {
                    Error::Io(inner_err)
                }
            }
            other => other,
        })
    }

    async fn get_object_stream(&self, s3_uri: &S3Uri) -> Result<ByteStream, Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} get request", key);

        self.storage
            .read_byte_stream(&key)
            .await
            .map_err(|err| match err {
                Error::Io(inner_err) => {
                    if inner_err.kind() == std::io::ErrorKind::NotFound {
                        Error::S3("Key doesn't exists".to_string())
                    } else {
                        Error::Io(inner_err)
                    }
                }
                other => other,
            })
    }

    async fn exists(&self, s3_uri: &S3Uri) -> Result<bool, Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} exists request", key);
        Ok(self.storage.exists(&key).await)
    }

    async fn put_object(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Result<(), Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} put request", key);
        let contents_vec = contents.into().collect().await?.to_vec();
        self.storage.write_file(key, &contents_vec).await
    }

    async fn put_object_and_checksum(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error> {
        let key = s3_uri.to_string();
        let contents_vec = contents.into().collect().await?.to_vec();
        self.storage.write_file(&key, &contents_vec).await?;

        let file = self.storage.open_file(&key).await?;
        let hash = checksum::calculate_sha256_chunked_checksum(file, size).await?;
        Ok((Some("version".to_string()), hash.to_vec()))
    }

    async fn multipart_upload_and_checksum(
        &self,
        _s3_uri: &S3Uri,
        _file_path: impl AsRef<Path>,
        _size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error> {
        Ok((None, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_object() -> Result<(), Error> {
        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://found/n?versionId=v")?,
                b"Hello".to_vec(),
            )
            .await?;
        let s3_uri_not_found = S3Uri::try_from("s3://b/n?versionId=v")?;
        let not_found = remote.get_object(&s3_uri_not_found).await;
        match not_found {
            Err(err) => assert_eq!(err.to_string(), "S3 error: Key doesn't exists".to_string()),
            Ok(_) => panic!("shouldn't happen"),
        }
        let s3_uri_found = S3Uri::try_from("s3://found/n?versionId=v")?;
        let mut found = remote.get_object(&s3_uri_found).await?;
        let mut output = Vec::new();
        found.read_to_end(&mut output).await?;
        assert_eq!(output, b"Hello");
        Ok(())
    }
}
