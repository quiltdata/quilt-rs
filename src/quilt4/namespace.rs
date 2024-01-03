//!
//! Namespace is a collection of Manifests within a Domain
//! that are accessed by a "prefix/suffix".
//! It uses a StringMap to convert tags (e.g., "latest")
//! or numerical timestamps into hashes
//! 

use super::{
    client::{Client, GetClient},
    domain::Domain,
    upath::UPath,
    string_map::{StringMap, StringIterator},
};

use async_trait::async_trait;
use object_store::path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Namespace<'a> {
    _client: &'a Client,
    path: UPath,
}

impl<'a> Namespace<'a> {
  pub fn new(_client: &'a Client, path: UPath) -> Self {
    Namespace {
      _client,
      path,
    }
  }

  pub fn timestamp() -> String {
      let now = SystemTime::now();
      let duration = now.duration_since(UNIX_EPOCH).expect("Failed to get duration");
      duration.as_secs().to_string()
  }

  pub fn to_string(&self) -> String {
    format!("Namespace({:?})", self.path)
  }

  pub async fn to_hash(&self, path: &UPath) -> String {
      let hash_result: Result<Vec<u8>, std::io::Error> = path.read_bytes(self.get_client()).await;
      let hash = hash_result.expect("Failed to read hash");
      String::from_utf8(hash).expect("Failed to convert hash to string")
  }

  pub async fn relax(&self, target_domain: &Domain<'a>, target_path: UPath) -> Self {
    // create a "relaxed" version of this Namespace in the target Domain
    // by copying all of the Manifests from this Namespace to the target Domain
    // and then returning a new Namespace object in the target Domain
    // that points to the copied Manifests
    unimplemented!("Namespace::relax{}@{}", target_domain.to_string(), target_path.to_string())
  }

}

impl GetClient for Namespace<'_> {
    fn get_client(&self) -> &Client {
        self._client
    }
}

#[async_trait]
impl<'a> StringMap<'a, String> for Namespace<'a> {
    async fn get(&self, tag: &str) -> Option<String> {
        let path: UPath = self.path.join(tag);
        let hash_result: Result<Vec<u8>, std::io::Error> = path.read_bytes(self.get_client()).await;
        let hash = hash_result.expect("Failed to read hash");
        let hash_string = String::from_utf8(hash).expect("Failed to convert hash to string");
        Some(hash_string)
    }

    async fn insert(&mut self, _key: &str, hash: String) {
      let tag_path = self.path.join(_key);
      tag_path.write_bytes(self.get_client(), hash.as_bytes()).await;
    }

    async fn iter(&self) -> StringIterator {
      // returns list of hashes for all tags in this Namespace
      let tag_paths = self.path.list(&self.get_client(), 1).await;
      let mut hashes: Vec<String> = Vec::new();
      for tag_path in tag_paths {
        let hash_string = self.to_hash(&tag_path).await;
        hashes.push(hash_string);
      }
      StringIterator::new(hashes)
    }

}

