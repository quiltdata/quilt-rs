//! 
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! It will eventually also support web and document stores.
//! 

use std::fmt;
use std::path::PathBuf;
use object_store::path::Path;
use std::io;
use multihash::Hash;
use multihash::ContentHash;

union UPath {
    file: PathBuf,
    object: Path,
}

impl UPath {
    pub async fn read_bytes(&self) -> io::Result<bytes> { unimplemented!() }
    pub async fn write_bytes(&self, input: bytes) -> io::Result<bytes> { unimplemented!() }

    pub async fn parent(&self) -> Option<UPath> {
        // TODO: Implement parent method
        unimplemented!()
    }

    pub async fn hash(&self, algorithm: ContentHash) -> Hash {
        // TODO: Implement hash method
        unimplemented!()
    }

    pub async fn is_folder(&self) -> bool {
        // TODO: Implement is_folder method
        unimplemented!()
    }
}
