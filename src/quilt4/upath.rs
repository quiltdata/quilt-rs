//! 
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! 

use std::fmt;
use std::path::PathBuf;
use object_store::path::Path;
use std::io;
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

    pub async fn hash(&self, algorithm: String) -> String {
        // TODO: Implement hash method
        unimplemented!()
    }

    pub async fn is_folder(&self) -> bool {
        // TODO: Implement is_folder method
        unimplemented!()
    }
}
