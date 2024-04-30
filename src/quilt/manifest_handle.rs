use std::path::PathBuf;

use crate::io::storage::Storage;
use crate::manifest::Table;
use crate::quilt::paths;
use crate::quilt::Error;
use crate::quilt::Namespace;
use crate::uri::ManifestUri;

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
    pub fn from_manifest_uri(
        manifest_uri: &ManifestUri,
        paths: &paths::DomainPaths,
    ) -> CachedManifest {
        CachedManifest {
            paths: paths.clone(),
            bucket: manifest_uri.bucket.clone(),
            hash: manifest_uri.hash.clone(),
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
