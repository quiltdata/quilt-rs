pub use crate::quilt::lineage::mocks as lineage;

pub mod manifest {
    use std::collections::BTreeMap;

    use crate::quilt::manifest_handle::ReadableManifest;
    use crate::quilt::storage::Storage;
    use crate::Error;
    use crate::Row4;
    use crate::Table;

    fn row4_with_name(name: String) -> Result<Row4, Error> {
        Ok(Row4 {
            name,
            place: "s3://z/x/y?versionId=foo".to_string(),
            hash: multihash::Multihash::wrap(345, b"Hello world")?,
            ..Row4::default()
        })
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

    pub fn with_record_keys(keys: Vec<String>) -> Result<impl ReadableManifest, Error> {
        struct InMemoryManifest {
            keys: Vec<String>,
        }
        impl ReadableManifest for InMemoryManifest {
            async fn read(&self, _storage: &mut impl Storage) -> Result<Table, Error> {
                let mut records = BTreeMap::new();
                for key in &self.keys {
                    records.insert(key.to_string(), row4_with_name(key.to_string())?);
                }
                Ok(Table {
                    records,
                    ..Table::default()
                })
            }
        }
        Ok(InMemoryManifest { keys })
    }
}
