//!
//! Namespace is a collection of Manifests within a Domain
//! that are accessed by a "prefix/suffix".
//! 

use super::{
    domain::Domain,
    upath::UPath,
    manifest::Manifest4,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Namespace<'a> {
    #[serde(skip)]
    _domain: &'a Domain,
    path: UPath,
}

impl Namespace {
    pub fn new(_domain: &Domain, path: UPath) -> Self {
        Namespace {
            _domain,
            path,
        }
    }

    pub fn to_string(&self) -> String {
        format!("Namespace({:?})^{}", self.path, self._domain.to_string())
    }

    pub async fn relax(&self, target_domain: &Domain) -> Self {
      /// create a "relaxed" version of this Namespace in the target Domain
      /// by copying all of the Manifests from this Namespace to the target Domain
      /// and then returning a new Namespace object in the target Domain
      /// that points to the copied Manifests
      unimplemented!("Namespace::relax")
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
