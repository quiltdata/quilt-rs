//!
//! The Quilt4 Client maintains all the persistent state across invocations,
//! and is the main entry point for the API.
//! 
//! It's responsible for:
//! - authentication
//! - abstracting away the various data stores
//! - converting URIs into different resources
//! 
//! Initially, it also wraps Project 4F's local domain to support quilt-server.

use std::collections::HashMap;
use std::convert::TryFrom;
use aws_sdk_s3::Error;
use aws_sdk_s3::Client as S3Client;
use aws_types::region::Region;
use tracing::info;

use crate::LocalDomain;
use crate::Manifest;
use crate::S3PackageURI;
use crate::Table;
use crate::UPath;
use crate::quilt::RemoteManifest;

use super::{
    domain::Domain, namespace::Namespace, manifest::Manifest4, entry::Entry4
};

pub trait GetClient {
    fn get_client(&self) -> &Client;
}

#[derive(Clone, Debug)]
pub struct Client {
    _s3_clients: HashMap<Region, S3Client>,
    // TODO: lock
    cache_domain: LocalDomain,
}

impl Client {
    pub fn new(cache_domain: LocalDomain) -> Self {
        Client {
            _s3_clients: HashMap::new(),
            cache_domain,
        }
    }

    pub fn cache_domain(&self) -> &LocalDomain {
        &self.cache_domain
    }

    pub fn to_string(&self) -> String {
        format!("Client({})", std::env::current_dir().unwrap().to_string_lossy())
    }

    pub async fn manifest3_from_uri(&self, uri_string: &str) -> Result<Manifest, Error> {
        let uri = S3PackageURI::try_from(uri_string).expect("Failed to parse URI");
        let manifest = self.cache_domain.browse_uri(&uri).await.expect("Failed to browse remote package");
        info!("manifest: {:#?}", manifest);
        Ok(manifest)
    }

    pub async fn domain_from_key(_registry_uri: &str) -> Option<Domain> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn domain_keys(&self) -> Vec<String> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn domain_objects(&self) -> Vec<Domain> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn domain_from_uri(_uri: &str) -> Option<Domain> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn namespace_from_uri(_uri: &str) -> Option<Namespace> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn manifest_from_uri(&self, uri_string: &str) -> Result<Manifest4, Error> {
        let uri = S3PackageURI::try_from(uri_string).expect("Failed to parse URI");
        let remote_manifest = RemoteManifest::resolve(&uri).await.expect("Failed to resolve URI");
        let cached = self.cache_domain.cache_remote_manifest(&remote_manifest).await.expect("Failed to cache the manifest");

        let path = self.cache_domain.manifest_cache_path(&cached.bucket, &cached.hash);
        let upath = UPath::Local(path);
        let table = Table::read_from_upath(&upath).await.expect("Failed to read the manifest");
        let manifest = Manifest4::new(upath, Some(table));

        Ok(manifest)
    }

    pub async fn entry_from_uri(_uri: &str) -> Option<Entry4> {
        // Implementation goes here
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[tokio::test]
    async fn test_client_new() {
        let domain = LocalDomain::new(PathBuf::new());
        assert!(Client::new(domain).to_string().contains("Client"));
    }
}
