//!
//! Domain is the top-level persistent resource.
//! It wraps a UPath containing both the "registry" (named manifests)
//! and the "store" (immutable blobs).

use super:: {
    client::{Client, GetClient},
    upath::UPath,
    manifest::Manifest4,
    namespace::Namespace,
    string_map::{StringMap, StringIterator},
};
use async_trait::async_trait;

#[derive(Clone, Debug)]
pub struct Domain<'a> {
    _client: &'a Client,
    path: UPath,
    names: UPath,
    manifests: UPath,
}

static NAMESPACE_LATEST: &'static str = "latest";
static NAMESPACE_PATH: &'static str = ".quilt/named_packages";
static MANIFEST_PATH: &'static str = ".quilt/packages";

impl<'a> Domain<'a> {
  pub fn new<'b>(_client: &'b Client, path: UPath) -> Self where 'b: 'a {
    let names = path.join(NAMESPACE_PATH);
    let manifests = path.join(MANIFEST_PATH);
    Domain {
      _client,
      path,
      names,
      manifests,
    }
  }

  pub fn to_string(&self) -> String {
    format!("Domain({:?})^{}", self.path, self._client.to_string())
  }

  pub async fn get_latest<'a1, 'a2, 'a3>(&'a2 self, name: &'a3 str) -> Option<Manifest4<'a>> where 'a2: 'a {
    let namespace = self.get(name).await.unwrap();
    let hash_value = namespace.get(&NAMESPACE_LATEST).await.unwrap();
    let manifest = self.get_manifest(&hash_value).await;
    manifest
  }

  pub async fn get_manifest<'a1, 'a2, 'a3>(&'a2 self, hash: &'a3 str) -> Option<Manifest4<'a>> where 'a2: 'a {
    let filename = format!("{}.parquet", hash);
    let path: UPath = self.manifests.join(&filename);
    let manifest = Manifest4::from_path(self.get_client(), path).await;
    manifest
    // FIXME: lifetime may not live long enough
  }

  pub async fn insert_manifest(&mut self, manifest: &Manifest4<'a>) {
      let key = manifest.hash();
      let man_path = self.manifests.join(&key);
      manifest.write4(self.get_client(), man_path).await;
  }
 
}

impl GetClient for Domain<'_> {
    fn get_client(&self) -> &Client {
        self._client
    }
}


// TODO: cache Namespace objects for reuse
#[async_trait]
impl<'a> StringMap<'a, Namespace<'a>> for Domain<'a> {
    async fn get(&self, _key: &str) -> Option<Namespace<'a>> where 'life0: 'a {
      let path: UPath = self.names.join(_key);
      let namespace = Namespace::new(&self.get_client(), path);
      Some(namespace)
    }

    async fn insert(&mut self, _key: &str, namespace: &Namespace) {
      let path: UPath = self.names.join(_key);
      unimplemented!("Domain::insert{}@{}", namespace.to_string(), path.to_string())
    }

    async fn iter(&self) -> StringIterator {
      let names = self.names.list(&self.get_client(), 2).await;
      let string_items = names.iter().map(|item| item.to_string()).collect();
      StringIterator::new(string_items)
    }
}
