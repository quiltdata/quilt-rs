use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::log;

use crate::io::remote::utils::bytestream_to_string;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::paths;
use crate::quilt::manifest::Manifest;
use crate::quilt::Namespace;
use crate::quilt::RevisionPointer;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::uri::TagUri;
use crate::Error;
use crate::Table;

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

async fn resolve_top_hash(remote: &impl Remote, uri: TagUri) -> Result<String, Error> {
    let stream = remote.get_object_stream(&uri.into()).await?;
    bytestream_to_string(stream).await
}

impl ManifestUri {
    pub async fn from_package_uri(remote: &impl Remote, uri: &S3PackageUri) -> Result<Self, Error> {
        // resolve the actual hash
        let top_hash = match &uri.revision {
            RevisionPointer::Hash(top_hash) => top_hash.clone(),
            RevisionPointer::Tag(_) => {
                let uri = TagUri::latest(&uri.into());
                resolve_top_hash(remote, uri).await?
            }
        };

        Ok(Self {
            bucket: uri.bucket.clone(),
            namespace: uri.namespace.clone(),
            hash: top_hash,
        })
    }

    pub async fn resolve_latest(&self, remote: &impl Remote) -> Result<String, Error> {
        let uri = TagUri::latest(self);
        resolve_top_hash(remote, uri).await
    }

    async fn put_tag(
        &self,
        remote: &impl Remote,
        tag_uri: TagUri,
        hash: &str,
    ) -> Result<(), Error> {
        remote
            .put_object(&tag_uri.into(), hash.as_bytes().to_vec())
            .await
    }

    pub async fn put_timestamp_tag(
        &self,
        remote: &impl Remote,
        timestamp: chrono::DateTime<chrono::Utc>,
        hash: &str,
    ) -> Result<(), Error> {
        let uri = TagUri::timestamp(self, timestamp);
        self.put_tag(remote, uri, hash).await
    }

    pub async fn update_latest(&self, remote: &impl Remote, hash: &str) -> Result<(), Error> {
        let uri = TagUri::latest(self);
        self.put_tag(remote, uri, hash).await
    }

    pub async fn upload_from(
        &self,
        storage: &impl Storage,
        remote: &impl Remote,
        manifest_path: &PathBuf,
    ) -> Result<(), Error> {
        // TODO: FAIL if the manifest with this hash already exists?
        let body = storage.read_byte_stream(manifest_path).await?;
        // let body = Manifest::from(&table).to_jsonlines().as_bytes().to_vec();
        let s3uri = S3Uri::from(self);
        log::info!("writing remote manifest to {}", s3uri.key);
        remote.put_object(&s3uri, body).await
    }

    pub async fn upload_legacy(&self, remote: &impl Remote, table: &Table) -> Result<(), Error> {
        let s3uri = S3Uri {
            bucket: self.bucket.clone(),
            key: paths::get_manifest_key_legacy(&self.hash),
            version: None,
        };
        remote
            .put_object(
                &s3uri,
                Manifest::from(table).to_jsonlines().as_bytes().to_vec(),
            )
            .await
    }
}

impl From<&ManifestUri> for S3Uri {
    fn from(remote: &ManifestUri) -> S3Uri {
        S3Uri {
            bucket: remote.bucket.clone(),
            key: paths::get_manifest_key(&remote.hash),
            version: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::quilt::mocks;

    #[tokio::test]
    async fn test_resolve_existing_hash() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@hjknlmn")?;
        let remote = mocks::remote::MockRemote::default();
        let manifest_uri = ManifestUri::from_package_uri(&remote, &uri).await?;
        assert_eq!(
            manifest_uri,
            ManifestUri {
                bucket: "b".to_string(),
                namespace: ("foo", "bar").into(),
                hash: "hjknlmn".to_string(),
            },
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_remote_hash() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar")?;
        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let manifest_uri = ManifestUri::from_package_uri(&remote, &uri).await?;
        assert_eq!(
            manifest_uri,
            ManifestUri {
                bucket: "b".to_string(),
                namespace: ("foo", "bar").into(),
                hash: "abcdef".to_string(),
            },
        );
        Ok(())
    }
}
