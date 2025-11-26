use std::path::Path;

use aws_sdk_s3::primitives::ByteStream;
use multihash::Multihash;
use tracing::log;

use crate::checksum;
use crate::error::S3Error;
use crate::io::remote::{HostConfig, RemoteObjectStream};
use crate::io::storage::mocks::MockStorage;
use crate::io::storage::Storage;
use crate::uri::Host;
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
    async fn exists(&self, _host: &Option<Host>, s3_uri: &S3Uri) -> Res<bool> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} exists request");
        Ok(self.storage.exists(&key).await)
    }

    async fn get_object_stream(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> Res<RemoteObjectStream> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} get request");

        let body = self
            .storage
            .read_byte_stream(&key)
            .await
            .map_err(|err| match err {
                // TODO: made a similar finer error for the ByteStreamError
                Error::ByteStreamError(_) => Error::S3(
                    host.to_owned(),
                    S3Error::GetObjectStream(
                        "NoSuchKey: The specified key does not exist".to_string(),
                    ),
                ),
                Error::Io(inner_err) => {
                    if inner_err.kind() == std::io::ErrorKind::NotFound {
                        Error::S3(
                            host.to_owned(),
                            S3Error::GetObjectStream(
                                "NoSuchKey: The specified key does not exist".to_string(),
                            ),
                        )
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

    async fn put_object(
        &self,
        _host: &Option<Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Res {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} put request");
        let contents_vec = contents.into().collect().await?.to_vec();
        self.storage.write_file(key, &contents_vec).await
    }

    async fn resolve_url(&self, host: &Option<Host>, s3_uri: &S3Uri) -> Res<S3Uri> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} HEAD request");
        if self.storage.exists(&key).await {
            Ok(s3_uri.clone())
        } else {
            Err(Error::S3(
                host.to_owned(),
                S3Error::ResolveUrl("NoSuchKey: The specified key does not exist".to_string()),
            ))
        }
    }

    async fn upload_file(
        &self,
        _host: &Option<Host>,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Res<(S3Uri, Multihash<256>)> {
        let file = self.storage.open_file(source_path.as_ref()).await?;
        let hash = checksum::Sha256ChunkedHash::from_async_read(file, size)
            .await?
            .into();
        Ok((
            S3Uri {
                version: Some("version".to_string()),
                ..dest_uri.clone()
            },
            hash,
        ))
    }

    async fn host_config(&self, _host: &Option<Host>) -> Res<HostConfig> {
        Ok(HostConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
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
        let not_found = remote.get_object_stream(&None, &s3_uri_not_found).await;
        if let Err(Error::S3(None, err)) = not_found {
            assert_eq!(
                err,
                S3Error::GetObjectStream("NoSuchKey: The specified key does not exist".to_string(),)
            );
        } else {
            panic!("shouldn't happen");
        }
        let s3_uri_found = S3Uri::try_from("s3://found/n?versionId=v")?;
        let found = remote.get_object_stream(&None, &s3_uri_found).await?;
        assert_eq!(found.body.collect().await?.to_vec(), b"Hello");
        Ok(())
    }
}
