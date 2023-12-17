//! 
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! It will eventually also support web and document stores.
//! 

use std::path::PathBuf;
use object_store::path::Path;
use std::io;
use multihash::Multihash;

use super::client::Client;

#[derive(Clone, Debug)]
// FIXME: This should be a union, not a struct
pub struct UPath {
    pub uri: String,
    pub file_path: Option<PathBuf>,
    pub object_path: Option<Path>,
    pub object_bucket: Option<String>,
}

impl UPath {
    pub fn new(uri: String) -> Self {
        if uri.starts_with("s3://") {
            // split on the first '/' after the bucket name
            let mut parts = uri.splitn(2, '/');
            let bucket = parts.next().unwrap();
            let path = parts.next().unwrap();
            return UPath {
                uri: uri.clone(),
                file_path: None,
                object_path: Some(Path::from(path)),
                object_bucket: Some(bucket.to_string()),
            }
        } else if uri.starts_with("file://"){
            let path = Some(PathBuf::from(uri.clone()));
            return UPath {
                uri: uri.clone(),
                file_path: path,
                object_path: None,
                object_bucket: None,
            }
        }
        panic!("UPath::new() failed to parse uri: {}", uri);
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
    fn test_new() {
        let local_uri = shared::local_uri_parquet();
        let upath = UPath::new(local_uri.clone());
        assert_eq!(upath.uri, local_uri);
        let upath_string = upath.to_string();
        assert_eq!(upath_string, format!("UPath({})", local_uri));
        assert_eq!(upath.object_path, None);
        assert_eq!(upath.file_path, Some(PathBuf::from(local_uri)));
    }
}