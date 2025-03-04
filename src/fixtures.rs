pub mod sample_file_1 {
    use std::path::PathBuf;

    use multihash::Multihash;

    use crate::checksum::ContentHash;
    use crate::lineage::PathState;
    use crate::manifest::Row;
    use crate::Res;

    pub fn row_hash() -> Res<Multihash<256>> {
        // This is a hash of fixtures/manifest.jsonl file
        ContentHash::SHA256Chunked("4ssEkl5yUwi0LCjnsOl3pJ6ZgtgD8o5a6K9ayFtKDQE=".to_string())
            .try_into()
    }

    pub fn path_state() -> Res<PathState> {
        Ok(PathState {
            hash: row_hash()?,
            ..PathState::default()
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

    use crate::Res;

    const TEST_LOCAL_PARQUET: &str = "fixtures/manifest.parquet";
    const TEST_LOCAL_PARQUET_CHECKSUMMED: &str = "fixtures/checksummed.parquet";
    const TEST_LOCAL_JSONL: &str = "fixtures/manifest.jsonl";

    pub const JSONL_HASH: &str = "3af08e839fec032c6804596d32932f6f0550abe8b9696c56ed15fe7f8e853ebd";

    fn local_uri(key: &str) -> Res<PathBuf> {
        Ok(std::env::current_dir()?.join(key))
    }

    pub fn parquet() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_PARQUET)
    }

    pub fn jsonl() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_JSONL)
    }

    pub fn parquet_checksummed() -> Res<PathBuf> {
        local_uri(TEST_LOCAL_PARQUET_CHECKSUMMED)
    }
}
