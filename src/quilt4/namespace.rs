//!
//! Namespace is a collection of Manifests within a Domain
//! that are accessed by a "prefix/suffix".
//! 

use super::{
    domain::Domain,
    upath::UPath,
    manifest::Manifest4,
};

#[derive(Clone, Debug)]
pub struct Namespace {
    _domain: Domain,
    path: UPath,
}

impl Namespace {
    pub async fn new(_domain: Domain, path: UPath) -> Self {
        Namespace {
            _domain,
            path,
        }
    }

    pub fn to_string(&self) -> String {
        format!("Namespace({})^{}", self.path.to_string(), self._domain.to_string())
    }

    pub async fn manifest_from_key(_manifest_tag: &str) -> Option<Manifest4> {
        // TODO: Implement stub for manifest_keys
        unimplemented!()
    }

    pub async fn manifest_keys(&self) -> Vec<String> {
        // TODO: Implement stub for manifest_keys
        unimplemented!()
    }

    pub async fn manifest_objects(&self) -> Vec<Manifest4> {
        // TODO: Implement stub for manifest_objects
        unimplemented!()
    }
}
