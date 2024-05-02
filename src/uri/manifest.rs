use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::log;

use crate::io::remote::utils::bytestream_to_string;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::Table;
use crate::paths;
use crate::uri::Namespace;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::uri::TagUri;
use crate::Error;

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

    use crate::mocks;

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
