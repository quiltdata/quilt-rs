use std::collections::HashMap;

use tokio::io::AsyncRead;
use tracing::log;

use crate::quilt::s3::S3Uri;
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
}
