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
use std::mem::ManuallyDrop;


#[derive(Clone)]
pub union UPath {
    object: ManuallyDrop<Path>,
    file: ManuallyDrop<PathBuf>,
}

impl UPath {
    pub fn new_object(path: Path) -> Self {
        Self { object: ManuallyDrop::new(path) }
    }

    pub fn new_file(path: PathBuf) -> Self {
        Self { file: ManuallyDrop::new(path) }
    }

    pub fn as_object(&self) -> &Path {
        unsafe { &*self.object }
    }

    pub fn as_file(&self) -> &PathBuf {
        unsafe { &*self.file }
    }

    pub async fn read_bytes(&self) -> io::Result<Vec<u8>> { unimplemented!() }
    pub async fn write_bytes(&self, input: Vec<u8>) -> io::Result<Vec<u8>> { unimplemented!() }

    pub async fn parent(&self) -> Option<UPath> {
        // TODO: Implement parent method
        unimplemented!()
    }

    pub async fn hash(&self, algorithm: String) -> Multihash<128> {
        // TODO: Implement hash method
        unimplemented!()
    }

    pub async fn is_folder(&self) -> bool {
        // TODO: Implement is_folder method
        unimplemented!()
    }


}