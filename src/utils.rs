use std::collections::HashMap;
use std::path::PathBuf;

pub static TEST_LOCAL_PARQUET: &str = "fixtures/manifest.parquet";
pub static TEST_LOCAL_PARQUET_CHECKSUMMED: &str = "fixtures/checksummed.parquet";
pub static TEST_LOCAL_JSONL: &str = "fixtures/manifest.jsonl";

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum TestFile {
    Parquet,
    ParquetChecksummed,
    Json,
    Domain,
}

pub fn local_uri(key: TestFile) -> PathBuf {
    let files: HashMap<TestFile, &str> = HashMap::from([
        (TestFile::Parquet, TEST_LOCAL_PARQUET),
        (TestFile::ParquetChecksummed, TEST_LOCAL_PARQUET_CHECKSUMMED),
        (TestFile::Json, TEST_LOCAL_JSONL),
        (TestFile::Domain, ""),
    ]);

    let cwd = std::env::current_dir().unwrap();
    cwd.join(files[&key])
}

pub fn local_uri_parquet() -> PathBuf {
    local_uri(TestFile::Parquet)
}

pub fn local_uri_json() -> PathBuf {
    local_uri(TestFile::Json)
}

pub fn local_uri_parquet_checksummed() -> PathBuf {
    local_uri(TestFile::ParquetChecksummed)
}
