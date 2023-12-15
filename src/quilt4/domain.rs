//!
//! Domain is the top-level persistent resource.
//! It wraps a UPath containing both the "registry" (named manifests)
//! and the "store" (immutable blobs).

use super:: {
    client::Client,
    upath::UPath,
    namespace::Namespace,
};

#[derive(Clone, Debug)]
pub struct Domain {
    _client: Client,
    path: UPath,
}

impl Domain {
    pub async fn new(_client: Client, path: UPath) -> Self {
        Domain {
            _client,
            path,
        }
    }

    #[allow(dead_code)]
    pub fn get_client(&self) -> &Client {
        &self._client
    }

    pub fn to_string(&self) -> String {
        format!("Domain({})^{}", self.path.to_string(), self._client.to_string())
    }        

    pub async fn namespace_from_key(_pkg_name: &str) -> Option<Namespace> {
        // TODO: Implement stub for namespace_from_key
        unimplemented!()
    }

    pub async fn namespace_keys(&self) -> Vec<String> {
        // TODO: Implement stub for namespace_keys
        unimplemented!()
    }

    pub async fn namespace_objects(&self) -> Vec<Namespace> {
        // TODO: Implement stub for namespace_objects
        unimplemented!()
    }
}
