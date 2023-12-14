//!
//! Namespace is a collection of Manifests within a Domain
//! that are accessed by a "prefix/suffix".
//! 

pub struct Namespace {
    parent: Domain,
    path: UPath,
}

impl Namespace {
    pub async fn new(parent: Domain, path: UPath) -> Self {
        Namespace {
            parent,
            path,
        }
    }

    pub async fn manifest_from_key(pkg_name: &str) -> Option<Manifest4> {
        // TODO: Implement stub for manifest_keys
        unimplemented!()
    }

    pub async fn manifest_keys(&self) -> Vec<String> {
        // TODO: Implement stub for manifest_keys
        unimplemented!()
    }

    pub async fn manifest_objects(&self, manifest: &str) -> Vec<Manifest4> {
        // TODO: Implement stub for manifest_objects
        unimplemented!()
    }
}
