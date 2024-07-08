use std::path::PathBuf;

use multihash::Multihash;

use crate::manifest::Place;
use crate::manifest::Row;

/// We use it for getting hashes in files listings when we create new packages from S3 directory.
/// Also, we re-use this struct for calculating hashes locally when S3-checksums are disabled.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: PathBuf,
    pub place: Place,
    pub size: u64,
    pub hash: Multihash<256>,
}

impl From<Entry> for Row {
    fn from(row: Entry) -> Self {
        Row {
            hash: row.hash,
            info: serde_json::Value::Null,
            meta: serde_json::Value::Null,
            name: row.name,
            place: row.place,
            size: row.size,
        }
    }
}
