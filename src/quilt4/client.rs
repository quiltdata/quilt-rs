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

pub struct Client {
    local_domain: LocalDomain,
    s3_clients: HashMap<String, S3Client>,
}

impl Client {
    pub async fn new() -> Self {
        cwd = std::env::current_dir().unwrap();
        Client {
            local_domain = LocalDomain::new(cwd),
            s3_clients: HashMap::new(),
        }
    }

    pub async fn manifest3_from_uri(uri_string: String) -> Result<Manifest, Error> {
        let uri = S3PackageURI::try_from(uri_string.as_str()).expect("Failed to parse URI");
        let manifest: Manifest = browse_remote_package(local_domain.into(), uri)
            .await
            .expect("Failed to browse remote package");
        println!("manifest: {:#?}", manifest);
        Ok(manifest)
    }

    pub async fn domain_from_key(registry_uri: &str) -> Option<Domain> {
        // TODO: Implement domain extraction logic
        None
    }

    pub async fn domain_keys(&self) -> Vec<String> {
        // TODO: Implement stub for domain_keys
        unimplemented!()
    }

    pub async fn domain_objects(&self, domain: &str) -> Vec<Domain> {
        // TODO: Implement stub for domain_objects
        unimplemented!()
    }

    pub async fn domain_from_uri(uri: &str) -> Option<Domain> {
        // TODO: Implement domain extraction logic
        None
    }

    pub async fn namespace_from_uri(uri: &str) -> Option<Namespace> {
        // TODO: Implement namespace extraction logic
        None
    }

    pub async fn manifest_from_uri(uri: &str) -> Option<Manifest4> {
        // TODO: Implement manifest extraction logic
        None
    }

    pub async fn entry_from_uri(uri: &str) -> Option<Entry4> {
        // TODO: Implement entry extraction logic
        None
    }
}