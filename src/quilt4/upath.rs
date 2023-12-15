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


#[derive(Clone, Debug)]
// FIXME: This should be a union, not a struct
pub struct UPath {
    object: Option<Path>,
    file: Option<PathBuf>,
}

impl UPath {
    pub fn to_string(&self) -> String {
        if self.object.is_some() {
            format!("UPath(object:{:?})", self.object)
        } else {
            format!("UPath(file:{:?})", self.file)
        }
    }

    pub async fn read_bytes(&self) -> io::Result<Vec<u8>> { unimplemented!() }
    pub async fn write_bytes(&self, _input: Vec<u8>) -> io::Result<Vec<u8>> { unimplemented!() }

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