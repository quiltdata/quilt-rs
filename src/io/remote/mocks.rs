use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::Object;
use multihash::Multihash;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tracing::log;

use crate::checksum;
use crate::io::remote::ObjectsStream;
use crate::io::remote::RemoteObjectStream;
use crate::io::remote::S3Attributes;
use crate::io::storage::mocks::MockStorage;
use crate::io::storage::Storage;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

use super::Remote;

/// A mock implementation of the `Remote` trait.
#[derive(Default)]
pub(crate) struct MockRemote {
    pub(crate) storage: MockStorage,
}

impl Remote for MockRemote {
    async fn exists(&self, s3_uri: &S3Uri) -> Res<bool> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} exists request", key);
        Ok(self.storage.exists(&key).await)
    }

    async fn get_object(&self, s3_uri: &S3Uri) -> Res<impl AsyncRead + Send + Unpin> {
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

    async fn get_object_attributes(
        &self,
        listing_uri: &S3Uri,
        object: &Object,
    ) -> Res<S3Attributes> {
        let key = object.key.clone().ok_or(Error::ObjectKey)?;
        let uri = S3Uri {
            bucket: listing_uri.bucket.clone(),
            key,
            version: None,
        };
        let stream = self.get_object_stream(&uri).await?;
        self.storage
            .get_object_attributes(stream, listing_uri, object)
            .await
    }

    async fn get_object_stream(&self, s3_uri: &S3Uri) -> Res<RemoteObjectStream> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} get request", key);

        let body = self
            .storage
            .read_byte_stream(&key)
            .await
            .map_err(|err| match err {
                // TODO: made a similar finer error for the ByteStreamError
                Error::ByteStreamError(_) => Error::S3("Key doesn't exists".to_string()),
                Error::Io(inner_err) => {
                    if inner_err.kind() == std::io::ErrorKind::NotFound {
                        Error::S3("Key doesn't exists".to_string())
                    } else {
                        Error::Io(inner_err)
                    }
                }
                other => other,
            });
        Ok(RemoteObjectStream {
            body: body?,
            uri: s3_uri.clone(),
        })
    }

    async fn list_objects(&self, _listing_uri: S3Uri) -> impl ObjectsStream {
        tokio_stream::iter(Vec::new())
    }

    async fn put_object(&self, s3_uri: &S3Uri, contents: impl Into<ByteStream>) -> Res {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} put request", key);
        let contents_vec = contents.into().collect().await?.to_vec();
        self.storage.write_file(key, &contents_vec).await
    }

    async fn upload_file(
        &self,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Res<(S3Uri, Multihash<256>)> {
        let file = self.storage.open_file(source_path.as_ref()).await?;
        let hash = checksum::calculate_sha256_chunked_checksum(file, size).await?;
        Ok((
            S3Uri {
                version: Some("version".to_string()),
                ..dest_uri.clone()
            },
            hash,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_object() -> Res {
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
