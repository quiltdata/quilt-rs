pub use crate::lineage::mocks as lineage;

pub use crate::io::remote::mocks as remote;

pub use crate::io::storage::mocks as storage;

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

    use crate::manifest::Row;
    use crate::manifest::Table;

    static TEST_LOCAL_PARQUET: &str = "fixtures/manifest.parquet";
    static TEST_LOCAL_PARQUET_CHECKSUMMED: &str = "fixtures/checksummed.parquet";
    static TEST_LOCAL_JSONL: &str = "fixtures/manifest.jsonl";

    pub const JSONL_HASH: &str = "3af08e839fec032c6804596d32932f6f0550abe8b9696c56ed15fe7f8e853ebd";

    fn local_uri(key: &str) -> PathBuf {
        std::env::current_dir().unwrap().join(key)
    }

    pub fn parquet() -> PathBuf {
        local_uri(TEST_LOCAL_PARQUET)
    }

    pub fn jsonl() -> PathBuf {
        local_uri(TEST_LOCAL_JSONL)
    }

    pub fn parquet_checksummed() -> PathBuf {
        local_uri(TEST_LOCAL_PARQUET_CHECKSUMMED)
    }

    pub fn row_with_name(name: PathBuf) -> Row {
        Row {
            name,
            place: "file:///z/x/y".to_string(),
            hash: row_hash_sample1(),
            ..Row::default()
        }
    }

    pub fn with_record_keys(keys: Vec<PathBuf>) -> Table {
        let mut table = Table::default();
        let mut records = BTreeMap::new();
        for key in &keys {
            records.insert(key.clone(), row_with_name(key.clone()));
        }
        table.set_records(records);
        table
    }

    pub fn with_rows(rows: Vec<Row>) -> Table {
        let mut table = Table::default();
        let mut records = BTreeMap::new();
        for row in &rows {
            records.insert(row.name.clone(), row.clone());
        }
        table.set_records(records);
        table
    }
}
