//!
//! Domain is the top-level persistent resource.
//! It wraps a UPath containing both the "registry" (named manifests)
//! and the "store" (immutable blobs).

use super:: {
    client::Client,
    upath::UPath,
    namespace::Namespace,
};

pub struct Domain {
    parent: Client,
    path: UPath,
}

impl Domain {
    pub async fn new(parent: Client, path: UPath) -> Self {
        Domain {
            parent,
            path,
        }
    }

    pub async fn namespace_from_key(pkg_name: &str) -> Option<Namespace> {
        // TODO: Implement stub for namespace_from_key
        unimplemented!()
    }

    pub async fn namespace_keys(&self) -> Vec<String> {
        // TODO: Implement stub for namespace_keys
        unimplemented!()
    }

    pub async fn namespace_objects(&self, namespace: &str) -> Vec<Namespace> {
        // TODO: Implement stub for namespace_objects
        unimplemented!()
    }
}
