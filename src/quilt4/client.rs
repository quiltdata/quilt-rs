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
use serde::{Deserialize, Serialize};
use aws_sdk_s3::Error;
use aws_sdk_s3::Client as S3Client;
use aws_types::region::Region;
use tracing::info;

use crate::LocalDomain;
use crate::Manifest;
use crate::S3PackageURI;

use super::{
    domain::Domain, namespace::Namespace, manifest::Manifest4, entry::Entry4
};

pub trait GetClient {
    fn get_client(&self) -> &Client;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Client {
    #[serde(skip)]
    _s3_clients: HashMap<Region, S3Client>,
}

impl Client {

    pub fn domain3() -> LocalDomain {
        let cwd = std::env::current_dir().unwrap();
        LocalDomain::new(cwd)
    }

    pub fn new() -> Self {
        Client {
            _s3_clients: HashMap::new(),
        }
    }


    pub fn to_string(&self) -> String {
        format!("Client({})", std::env::current_dir().unwrap().to_string_lossy())
    }        

    pub async fn manifest3_from_uri(&self, uri_string: &str) -> Result<Manifest, Error> {
        let uri = S3PackageURI::try_from(uri_string).expect("Failed to parse URI");
        let local = Client::domain3();
        let manifest = local.browse_uri(&uri).await.expect("Failed to browse remote package");
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

    pub async fn manifest_from_uri(_uri: &str) -> Option<Manifest4> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn entry_from_uri(_uri: &str) -> Option<Entry4> {
        // Implementation goes here
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_new() {
        assert!(Client::new().to_string().contains("Client"));
    }
}
