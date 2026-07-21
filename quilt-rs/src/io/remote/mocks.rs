use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use aws_sdk_s3::primitives::ByteStream;
use tracing::log;

use crate::Error;
use crate::error::FsError;
use crate::error::S3Error;
use crate::error::S3ErrorKind;
use crate::io::remote::HostConfig;
use crate::io::remote::RemoteObjectStream;
use crate::io::storage::Storage;
use crate::io::storage::mocks::MockStorage;
use crate::object_hash::ObjectHash;
use crate::object_hash::Sha256ChunkedHash;
use quilt_uri::Host;
use quilt_uri::S3Uri;

use crate::Res;

use super::Remote;

/// A mock implementation of the `Remote` trait.
#[derive(Default)]
pub struct MockRemote {
    pub(crate) storage: MockStorage,
    /// Per-URI count of `get_object_stream` calls, so tests can assert that a
    /// config or schema document is fetched exactly once across an operation.
    get_object_calls: Arc<Mutex<HashMap<String, usize>>>,
}

impl MockRemote {
    /// How many times `get_object_stream` was called for `uri`.
    ///
    /// # Panics
    ///
    /// Panics if the internal call-count mutex is poisoned.
    #[must_use]
    pub fn get_object_count(&self, uri: &str) -> usize {
        self.get_object_calls
            .lock()
            .unwrap()
            .get(uri)
            .copied()
            .unwrap_or(0)
    }
}

impl Remote for MockRemote {
    async fn exists(&self, _host: &Option<Host>, s3_uri: &S3Uri) -> Res<bool> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} exists request");
        Ok(self.storage.exists(&key).await)
    }

    async fn get_object_stream(
        &self,
        _host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> Res<RemoteObjectStream> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} get request");
        *self
            .get_object_calls
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_insert(0) += 1;

        let body = self
            .storage
            .read_byte_stream(&key)
            .await
            .map_err(|err| match err {
                Error::Fs(FsError::ByteStream(_)) => {
                    S3Error::new(S3ErrorKind::NotFound(key.clone())).into()
                }
                Error::Io(inner_err) if inner_err.kind() == std::io::ErrorKind::NotFound => {
                    S3Error::new(S3ErrorKind::NotFound(key.clone())).into()
                }
                other => other,
            });
        Ok(RemoteObjectStream {
            body: body?,
            uri: s3_uri.clone(),
        })
    }

    async fn put_object(
        &self,
        _host: &Option<Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Res {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} put request");
        self.storage.write_byte_stream(key, contents.into()).await
    }

    async fn resolve_url(&self, _host: &Option<Host>, s3_uri: &S3Uri) -> Res<S3Uri> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} HEAD request");
        if self.storage.exists(&key).await {
            Ok(s3_uri.clone())
        } else {
            Err(Error::S3(S3Error::new(S3ErrorKind::NotFound(key))))
        }
    }

    async fn upload_file(
        &self,
        _host_config: &HostConfig,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Res<(S3Uri, ObjectHash)> {
        let file = self.storage.open_file(source_path.as_ref()).await?;
        let hash = Sha256ChunkedHash::from_async_read(file, size).await?;
        Ok((
            S3Uri {
                version: Some("version".to_string()),
                ..dest_uri.clone()
            },
            hash.into(),
        ))
    }

    async fn host_config(&self, _host: &Option<Host>) -> Res<HostConfig> {
        Ok(HostConfig::default())
    }

    async fn verify_bucket(&self, _bucket: &str) -> Res {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_get_object_stream() -> Res {
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://found/n?versionId=v")?,
                b"Hello".to_vec(),
            )
            .await?;
        let s3_uri_not_found = S3Uri::try_from("s3://b/n?versionId=v")?;
        let Err(err) = remote.get_object_stream(&None, &s3_uri_not_found).await else {
            panic!("expected S3NotFound error");
        };
        assert!(err.is_not_found(), "expected S3NotFound, got: {err}");
        let s3_uri_found = S3Uri::try_from("s3://found/n?versionId=v")?;
        let found = remote.get_object_stream(&None, &s3_uri_found).await?;
        assert_eq!(found.body.collect().await?.to_vec(), b"Hello");
        Ok(())
    }
}
