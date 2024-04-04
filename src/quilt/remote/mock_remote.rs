use std::collections::HashMap;

use tokio::io::AsyncRead;

use crate::Error;

use super::Remote;

/// A mock implementation of the `Remote` trait.
#[derive(Default)]
pub(crate) struct MockRemote {
    pub(crate) registry: HashMap<String, Vec<u8>>,
}

impl Remote for MockRemote {
    async fn get_object(&self, bucket: &str, key: &str) -> Result<impl AsyncRead + Send + Unpin, Error> {
        match self.registry.get(&format!("s3://{}/{}", bucket, key)) {
            Some(vec) => Ok(vec.as_slice()),
            None => Err(Error::S3("Key doesn't exists".to_string())),
        }
    }

    async fn exists(&self, bucket: &str, key: &str) -> Result<bool, Error> {
        Ok(self
            .registry
            .contains_key(&format!("s3://{}/{}", bucket, key)))
    }
}
