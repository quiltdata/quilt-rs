pub use crate::quilt::lineage::mocks as lineage;

pub mod status {
    use multihash::Multihash;

    use crate::quilt::flow::status::PackageFileFingerprint;

    pub fn package_file_fingerprint() -> PackageFileFingerprint {
        PackageFileFingerprint {
            size: 0,
            hash: Multihash::wrap(0xb510, b"pedestrian").unwrap(),
        }
    }
}

pub mod manifest {
    use std::collections::BTreeMap;

    use multihash::Multihash;

    use crate::quilt::manifest_handle::ReadableManifest;
    use crate::quilt::storage::Storage;
    use crate::Error;
    use crate::Row4;
    use crate::Table;

    pub fn row4_with_name(name: String) -> Row4 {
        Row4 {
            name,
            place: "file:///z/x/y".to_string(),
            hash: Multihash::wrap(0xb510, b"pedestrian").unwrap(),
            ..Row4::default()
        }
    }

    pub fn default() -> impl ReadableManifest {
        struct InMemoryManifest {}
        impl ReadableManifest for InMemoryManifest {
            async fn read(&self, _storage: &mut impl Storage) -> Result<Table, Error> {
                Ok(Table::default())
            }
        }
        InMemoryManifest {}
    }

    pub fn with_record_keys(keys: Vec<String>) -> impl ReadableManifest {
        struct InMemoryManifest {
            keys: Vec<String>,
        }
        impl ReadableManifest for InMemoryManifest {
            async fn read(&self, _storage: &mut impl Storage) -> Result<Table, Error> {
                let mut records = BTreeMap::new();
                for key in &self.keys {
                    records.insert(key.to_string(), row4_with_name(key.to_string()));
                }
                Ok(Table {
                    records,
                    ..Table::default()
                })
            }
        }
        InMemoryManifest { keys }
    }

    pub fn with_rows(rows: Vec<Row4>) -> impl ReadableManifest {
        struct InMemoryManifest {
            rows: Vec<Row4>,
        }
        impl ReadableManifest for InMemoryManifest {
            async fn read(&self, _storage: &mut impl Storage) -> Result<Table, Error> {
                let mut records = BTreeMap::new();
                for row in &self.rows {
                    records.insert(row.name.to_string(), row.clone());
                }
                Ok(Table {
                    records,
                    ..Table::default()
                })
            }
        }
        InMemoryManifest { rows }
    }
}
