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
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::Error;
use aws_types::region::Region;
use std::convert::TryFrom;

use crate::api::LocalDomain;
use crate::api::Manifest;
use crate::api::S3PackageURI;
use crate::api::browse_remote_package;

use super::{
    domain::Domain, namespace::Namespace, manifest::Manifest4, entry::Entry4
};

pub struct Client {
    s3_clients: HashMap<Region, S3Client>,
}

impl Client {

    pub fn local_domain() -> LocalDomain {
        let cwd = std::env::current_dir().unwrap();
        LocalDomain::new(cwd)
    }

    pub async fn new() -> Self {
        let cwd = std::env::current_dir().unwrap();
        Client {
            s3_clients: HashMap::new(),
        }
    }

    pub async fn manifest3_from_uri(&self, uri_string: String) -> Result<Manifest, Error> {
        let uri = S3PackageURI::try_from(uri_string.as_str()).expect("Failed to parse URI");
        let local = Client::local_domain().into();
        let manifest: Manifest = browse_remote_package(local, uri)
            .await
            .expect("Failed to browse remote package");
        println!("manifest: {:#?}", manifest);
        Ok(manifest)
    }

    pub async fn domain_from_key(registry_uri: &str) -> Option<Domain> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn domain_keys(&self) -> Vec<String> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn domain_objects(&self, domain: &str) -> Vec<Domain> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn domain_from_uri(uri: &str) -> Option<Domain> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn namespace_from_uri(uri: &str) -> Option<Namespace> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn manifest_from_uri(uri: &str) -> Option<Manifest4> {
        // Implementation goes here
        unimplemented!()
    }

    pub async fn entry_from_uri(uri: &str) -> Option<Entry4> {
        // Implementation goes here
        unimplemented!()
    }
}