use std::collections::HashMap;

use aws_sdk_s3::primitives::ByteStream;
use std::io::Write;
use tokio::io::AsyncRead;
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
}
