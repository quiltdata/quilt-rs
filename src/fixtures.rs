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

    pub const JSONL_HASH: &str = "0428ab8c8b0fe83d9e57fb6b26ff190173caad00ed7aeb683ce26cc4b56ea4bb";
    pub const PARQUEST_CHECKSUMMED_HASH: &str =
        "9c4db11437f11c3bbe25b39601069b8ed09b39f5f18ac29a13df4361240859d9";

    pub const PAQUET_CHECKSUMMED_HEADER_ONLY_HASH: &str =
        "39ee9fb46019db2d8373c991d7881ba90bbb6a6c65417e108c295363794dec3c";

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
