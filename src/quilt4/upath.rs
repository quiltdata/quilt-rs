//! 
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! It will eventually also support web and document stores.
//! 

use std::path::PathBuf;
use object_store::{
    aws::{resolve_bucket_region, AmazonS3Builder},
    path::Path,
    ClientOptions, GetOptions, ObjectStore,
};
use std::io;
use multihash::Multihash;

use super::client::Client;

#[derive(Clone, Debug)]
// FIXME: This should be a union, not a struct
pub struct UPath {
    uri: String,
    object: Option<Path>,
    file: Option<PathBuf>,
}

impl UPath {
    pub fn new(uri: String) -> Self {
        UPath {
            uri: uri.clone(),
            object: None,
            file: None,
        }
    }

    pub fn to_string(&self) -> String {
        if self.object.is_some() {
            format!("UPath(object:{:?})", self.object)
        } else {
            format!("UPath(file:{:?})", self.file)
        }
    }

    pub async fn read_bytes(&self, _client: Client) -> io::Result<Vec<u8>> { unimplemented!() }
    pub async fn write_bytes(&self, _client: Client, _input: Vec<u8>) -> io::Result<Vec<u8>> { unimplemented!() }

    pub async fn parent(&self) -> Option<UPath> {
        // TODO: Implement parent method
        unimplemented!()
    }

    pub async fn hash(&self, _algorithm: String) -> Multihash<128> {
        // TODO: Implement hash method
        unimplemented!()
    }

    pub async fn is_folder(&self) -> bool {
        // TODO: Implement is_folder method
        unimplemented!()
    }


}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_new() {
        let local = shared::TEST_DOMAIN;
        let upath = UPath::new("s3://my-bucket/path/to/file".to_string());
        assert_eq!(upath.uri, "s3://my-bucket/path/to/file".to_string());
    }
}