//! 
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! It will eventually also support web and document stores.
//! 

use std::path::PathBuf;
// use object_store::path::Path;
use std::io;
use multihash::Multihash;
use serde::{Deserialize, Serialize};
use aptos_openapi_link::impl_poem_type;
impl_poem_type!(UPath, "object", ());

use super::client::Client;
use super::uri::UriParser;

#[derive(Clone, Debug, Deserialize, Serialize)]
// FIXME: This should be a union, not a struct
pub struct UPath {
    pub uri: String,
    pub file_path: Option<PathBuf>,
    pub object_path: Option<String>,
    pub object_bucket: Option<String>,
}

impl UPath {
    pub fn new(uri_string: String) -> Self {
        let uri = UriParser::try_from(&uri_string).unwrap();
        let file_path = if uri.scheme == "file" {
            Some(PathBuf::from(uri.path.clone()))
        } else {
            None
        };
        let object_path = if uri.scheme == "s3" {
            Some(uri.path.clone()) // Path::from
        } else {
            None
        };
        let object_bucket = if uri.scheme == "s3" {
            Some(uri.host)
        } else {
            None
        };
        UPath {
            uri: uri_string,
            file_path,
            object_path,
            object_bucket,
        }
    }

    pub fn to_string(&self) -> String {
        format!("UPath({})", self.uri)
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
    fn test_new_local() {
        let local_uri = utils::local_uri_parquet();
        let upath = UPath::new(local_uri.clone());
        println!("upath: {:?}", upath);
        assert_eq!(upath.uri, local_uri);
        let upath_string = upath.to_string();
        assert_eq!(upath_string, format!("UPath({})", local_uri));
        assert_eq!(upath.object_bucket, None);
        assert_eq!(upath.object_path, None);
        let fpath = upath.file_path.unwrap();
        assert_eq!(local_uri, format!("file://{}", fpath.to_string_lossy()));
    }
}