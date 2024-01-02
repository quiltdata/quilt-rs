//!
//! Domain is the top-level persistent resource.
//! It wraps a UPath containing both the "registry" (named manifests)
//! and the "store" (immutable blobs).

use super:: {
    client::Client,
    upath::UPath,
    namespace::Namespace,
    string_map::{StringMap, StringIterator},
};
use async_trait::async_trait;

#[derive(Clone, Debug)]
pub struct Domain {
    _client: Client,
    path: UPath,
    names: UPath,
}

static NAMESPACE_PATH: &'static str = ".quilt/named_packages";
impl Domain {
    pub fn new(_client: Client, path: UPath) -> Self {
        let names = path.join(NAMESPACE_PATH);
        Domain {
            _client,
            path,
            names,
        }
    }

    pub fn to_string(&self) -> String {
        format!("Domain({:?})^{}", self.path, self._client.to_string())
    }        
}


// TODO: cache Namespace objects for reuse
#[async_trait]
impl<'a> StringMap<'a, Namespace<'a>> for Domain {
    async fn get(&self, _key: &str) -> Option<Namespace> {
      let path: UPath = self.names.join(_key);
      let namespace = Namespace::new(self, path);
      Some(namespace)
    }

    async fn insert(&mut self, _key: &str, namespace: &Namespace) {
      let path: UPath = self.names.join(_key);
      namespace.relax(&self).await;
    }

    async fn iter(&self) -> StringIterator {
      let items = self.names.list(self._client, 2).await;
      let string_items = items.iter().map(|item| item.to_string()).collect();
      StringIterator::new(string_items)
    }
}
