use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use multihash::Multihash;
use tracing::log;

use crate::checksum::calculate_sha256_chunked_checksum;
use crate::io::remote::s3::get_relative_name;
use crate::io::remote::GetObject;
use crate::io::remote::HeadObject;
use crate::io::remote::ObjectsStream;
use crate::io::storage::mocks::MockStorage;
use crate::io::storage::Storage;
use crate::io::Entry;
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

    async fn head_object(&self, s3_uri: &S3Uri) -> Res<HeadObject> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {} head request", key);
        let file = self
            .storage
            .open_file(&key)
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
            })?;
        let size = file.metadata().await?.len();
        Ok(HeadObject {
            size,
            version: None,
        })
    }

    async fn get_object_attributes(
        &self,
        listing_uri: &S3Uri,
        object_key: impl AsRef<str>,
    ) -> Res<Entry> {
        log::debug!(
            "Mocking {} get object attributes request",
            object_key.as_ref()
        );
        let object_uri = S3Uri {
            bucket: listing_uri.bucket.clone(),
            key: object_key.as_ref().to_string(),
            version: None, // FIXME: Where is version?
        };
        let name = get_relative_name(listing_uri, &object_uri);
        let key = object_uri.to_string();
        log::debug!("Mocking {} head request", key);
        let file = self.storage.open_file(&key).await?;
        let size = file.metadata().await?.len();
        let hash = calculate_sha256_chunked_checksum(file, size).await?;
        Ok(Entry {
            name,
            place: object_uri.into(),
            size,
            hash,
        })
    }

    async fn get_object_attributes_fallback(
        &self,
        listing_uri: &S3Uri,
        object_key: impl AsRef<str>,
    ) -> Res<Entry> {
        self.get_object_attributes(listing_uri, object_key).await
    }

    async fn get_object(&self, s3_uri: &S3Uri) -> Res<GetObject> {
        let head = self.head_object(s3_uri).await?;

        let key = s3_uri.to_string();
        log::debug!("Mocking {} get request", key);
        let stream = self
            .storage
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
            })?;

        Ok(GetObject { head, stream })
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
        let hash = calculate_sha256_chunked_checksum(file, size).await?;
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
        let found = remote
            .get_object(&s3_uri_found)
            .await?
            .stream
            .collect()
            .await?
            .to_vec();
        assert_eq!(found, b"Hello");
        Ok(())
    }
}
