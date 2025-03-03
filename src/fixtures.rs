pub mod sample_file_1 {
    use std::path::PathBuf;

    use crate::lineage::PackageFileFingerprint;
    use crate::lineage::PathState;
    use crate::manifest::Row;
    use crate::Res;

    pub fn row_hash() -> Res<multihash::Multihash<256>> {
        Ok(multihash::Multihash::wrap(0xb510, b"pedestrian")?)
    }

    pub fn path_state() -> Res<PathState> {
        Ok(PathState {
            hash: row_hash()?,
            ..PathState::default()
        })
    }

    pub fn fingerprint() -> Res<PackageFileFingerprint> {
        Ok(PackageFileFingerprint {
            size: 0,
            hash: row_hash()?,
        })
    }

    pub fn row(name: PathBuf) -> Res<Row> {
        Ok(Row {
            name,
            place: "file:///z/x/y".into(),
            hash: row_hash()?,
            ..Row::default()
        })
    }
}

pub mod manifest {
    use std::path::PathBuf;

    const TEST_LOCAL_PARQUET: &str = "fixtures/manifest.parquet";
    const TEST_LOCAL_PARQUET_CHECKSUMMED: &str = "fixtures/checksummed.parquet";
    const TEST_LOCAL_JSONL: &str = "fixtures/manifest.jsonl";

    pub const JSONL_HASH: &str = "3af08e839fec032c6804596d32932f6f0550abe8b9696c56ed15fe7f8e853ebd";

    fn local_uri(key: &str) -> PathBuf {
        std::env::current_dir().expect("Failed to get current directory").join(key)
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
}
