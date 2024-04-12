use std::collections::HashMap;
use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use std::io::Write;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tracing::log;

use crate::quilt::s3::S3Uri;
use crate::quilt4::checksum;
use crate::Error;

use super::Remote;

/// A mock implementation of the `Remote` trait.
#[derive(Default)]
pub(crate) struct MockRemote {
    pub(crate) registry: HashMap<String, Vec<u8>>,
}

impl MockRemote {
    pub(crate) fn new(registry: HashMap<String, Vec<u8>>) -> Self {
        MockRemote { registry }
    }
}

// TODO: instead of `&mut self` use MockStorage with temp data
impl Remote for MockRemote {
    async fn get_object(&self, s3_uri: &S3Uri) -> Result<impl AsyncRead + Send + Unpin, Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} get request", key);
        match self.registry.get(&key) {
            Some(vec) => Ok(vec.as_slice()),
            None => Err(Error::S3("Key doesn't exists".to_string())),
        }
    }

    async fn exists(&self, s3_uri: &S3Uri) -> Result<bool, Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} exists request", key);
        Ok(self.registry.contains_key(&key))
    }

    async fn put_object(
        &mut self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Result<(), Error> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} put request", key);
        self.registry
            .insert(key, contents.into().collect().await?.to_vec());
        Ok(())
    }

    async fn put_object_and_checksum(
        &mut self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error> {
        let key = s3_uri.to_string();
        let contents_vec = contents.into().collect().await?.to_vec();

        let mut temp_file = tempfile::tempfile()?;
        temp_file.write_all(&contents_vec)?;
        let file = tokio::fs::File::from_std(temp_file);
        let hash = checksum::calculate_sha256_chunked_checksum(file, size).await?;

        self.registry.insert(key, contents_vec);
        Ok((Some("version".to_string()), hash.to_vec()))
    }

    async fn multipart_upload_and_checksum(
        &mut self,
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

    // use std::collections::HashMap;

    #[tokio::test]
    async fn test_get_object() -> Result<(), Error> {
        let remote = MockRemote {
            registry: HashMap::from([("s3://found/n?versionId=v".to_string(), b"Hello".to_vec())]),
        };
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
