use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::log;

use crate::io::remote::Remote;
use crate::io::s3;
use crate::io::storage::Storage;
use crate::quilt::manifest::Manifest;
use crate::quilt::paths;
use crate::quilt::uri::Namespace;
use crate::quilt::uri::RevisionPointer;
use crate::quilt::uri::S3PackageUri;
use crate::quilt::Error;
use crate::quilt::Table;

pub fn tag_uri(bucket: &str, namespace: &Namespace, tag: &str) -> s3::S3Uri {
    s3::S3Uri {
        bucket: bucket.to_owned(),
        key: paths::tag_key(namespace, tag),
        version: None,
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteManifest {
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

impl RemoteManifest {
    pub async fn resolve(remote: &impl Remote, uri: &S3PackageUri) -> Result<Self, Error> {
        // resolve the actual hash
        let top_hash = match &uri.revision {
            RevisionPointer::Hash(top_hash) => top_hash.clone(),
            RevisionPointer::Tag(tag) => {
                tag_uri(&uri.bucket, &uri.namespace, tag)
                    .get_contents(remote)
                    .await?
            }
        };

        Ok(Self {
            bucket: uri.bucket.clone(),
            namespace: uri.namespace.clone(),
            hash: top_hash,
        })
    }

    pub async fn resolve_latest(&self, remote: &impl Remote) -> Result<String, Error> {
        tag_uri(&self.bucket, &self.namespace, "latest")
            .get_contents(remote)
            .await
    }

    async fn put_tag(&self, remote: &impl Remote, tag: &str, hash: &str) -> Result<(), Error> {
        tag_uri(&self.bucket, &self.namespace, tag)
            .put_contents(remote, hash.as_bytes().to_vec())
            .await
    }

    pub async fn put_timestamp_tag(
        &self,
        remote: &impl Remote,
        timestamp: chrono::DateTime<chrono::Utc>,
        hash: &str,
    ) -> Result<(), Error> {
        self.put_tag(remote, &timestamp.timestamp().to_string(), hash)
            .await
    }

    pub async fn update_latest(&self, remote: &impl Remote, hash: &str) -> Result<(), Error> {
        self.put_tag(remote, "latest", hash).await
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
        let s3uri = s3::S3Uri::from(self);
        log::info!("writing remote manifest to {}", s3uri.key);

        s3uri.put_contents(remote, body).await
    }

    pub async fn upload_legacy(&self, remote: &impl Remote, table: &Table) -> Result<(), Error> {
        let s3uri = s3::S3Uri {
            bucket: self.bucket.clone(),
            key: paths::get_manifest_key_legacy(&self.hash),
            version: None,
        };

        s3uri
            .put_contents(
                remote,
                Manifest::from(table).to_jsonlines().as_bytes().to_vec(),
            )
            .await
    }
}

impl From<&RemoteManifest> for s3::S3Uri {
    fn from(remote: &RemoteManifest) -> s3::S3Uri {
        s3::S3Uri {
            bucket: remote.bucket.clone(),
            key: paths::get_manifest_key(&remote.hash),
            version: None,
        }
    }
}

pub trait ReadableManifest {
    fn get_path_buf(&self) -> PathBuf {
        PathBuf::default()
    }

    fn read(
        &self,
        storage: &(impl Storage + Sync),
    ) -> impl std::future::Future<Output = Result<Table, Error>> + Send
    where
        Self: Sync,
    {
        async {
            let pathbuf = self.get_path_buf();
            let table = Table::read_from_path(storage, &pathbuf).await?;
            Ok(table)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CachedManifest {
    pub bucket: String,
    pub hash: String,
    pub paths: paths::DomainPaths,
}

impl ReadableManifest for CachedManifest {
    fn get_path_buf(&self) -> PathBuf {
        self.paths.manifest_cache(&self.bucket, &self.hash)
    }
}

impl CachedManifest {
    pub fn from_remote_manifest(
        remote_manifest: &RemoteManifest,
        paths: &paths::DomainPaths,
    ) -> CachedManifest {
        CachedManifest {
            paths: paths.clone(),
            bucket: remote_manifest.bucket.clone(),
            hash: remote_manifest.hash.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstalledManifest {
    pub hash: String,
    pub namespace: Namespace,
    pub paths: paths::DomainPaths,
}

impl ReadableManifest for InstalledManifest {
    fn get_path_buf(&self) -> PathBuf {
        self.paths.installed_manifest(&self.namespace, &self.hash)
    }
}

impl InstalledManifest {
    pub fn new(namespace: Namespace, hash: String, paths: paths::DomainPaths) -> Self {
        InstalledManifest {
            hash,
            namespace,
            paths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::s3::S3Uri;

    #[tokio::test]
    async fn test_resolve_existing_hash() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@hjknlmn")?;
        let remote = MockRemote::default();
        let remote_manifest = RemoteManifest::resolve(&remote, &uri).await?;
        assert_eq!(
            remote_manifest,
            RemoteManifest {
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
        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let remote_manifest = RemoteManifest::resolve(&remote, &uri).await?;
        assert_eq!(
            remote_manifest,
            RemoteManifest {
                bucket: "b".to_string(),
                namespace: ("foo", "bar").into(),
                hash: "abcdef".to_string(),
            },
        );
        Ok(())
    }
}
