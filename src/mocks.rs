pub use crate::lineage::mocks as lineage;

pub use crate::io::remote::mocks as remote;

pub use crate::io::storage::mocks as storage;

// TODO: move to src/mocks.rs

pub fn row_hash_sample1() -> multihash::Multihash<256> {
    multihash::Multihash::wrap(0xb510, b"pedestrian").expect("Unexpected")
}

pub mod status {
    use super::row_hash_sample1;

    use crate::lineage::PackageFileFingerprint;

    pub fn package_file_fingerprint() -> PackageFileFingerprint {
        PackageFileFingerprint {
            size: 0,
            hash: row_hash_sample1(),
        }
    }
}

pub mod manifest {
    use super::row_hash_sample1;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::io::storage::Storage;
    use crate::manifest::Row;
    use crate::manifest::Table;
    use crate::quilt::manifest_handle::ReadableManifest;
    use crate::Error;

    pub fn row_with_name(name: PathBuf) -> Row {
        Row {
            name,
            place: "file:///z/x/y".to_string(),
            hash: row_hash_sample1(),
            ..Row::default()
        }
    }

    pub fn default() -> impl ReadableManifest {
        struct InMemoryManifest {}
        impl ReadableManifest for InMemoryManifest {
            async fn read(&self, _storage: &impl Storage) -> Result<Table, Error> {
                Ok(Table::default())
            }
        }
        InMemoryManifest {}
    }

    pub fn with_record_keys(keys: Vec<PathBuf>) -> Table {
        let mut records = BTreeMap::new();
        for key in &keys {
            records.insert(key.clone(), row_with_name(key.clone()));
        }
        Table {
            records,
            ..Table::default()
        }
    }

    pub fn with_rows(rows: Vec<Row>) -> Table {
        let mut records = BTreeMap::new();
        for row in &rows {
            records.insert(row.name.clone(), row.clone());
        }
        Table {
            records,
            ..Table::default()
        }
    }
}
