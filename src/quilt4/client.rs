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

    pub async fn domain_from_key(key: &str) -> Option<Domain> {
        // TODO: Implement domain extraction logic
        None
    }
      
    pub async fn domain_from_uri(uri: &str) -> Option<Domain> {
        // TODO: Implement domain extraction logic
        None
    }

    pub async fn namespace_from_uri(uri: &str) -> Option<Namespace> {
        // TODO: Implement namespace extraction logic
        None
    }


    pub async fn manifest_from_uri(uri_string: String) -> Result<Manifest4, Error> {
        let uri = S3PackageURI::try_from(uri_string.as_str()).expect("Failed to parse URI");
        let manifest: Manifest = browse_remote_package(local_domain.into(), uri)
            .await
            .expect("Failed to browse remote package");
        println!("manifest: {:#?}", manifest);
        assert!(manifest.rows.len() > 0);
        manifest.rows.len();
        Ok(manifest)
    }

    pub async fn entry_from_uri(uri: &str) -> Option<String> {
        // TODO: Implement entry extraction logic
        None
    }
}