use std::collections::HashMap;
use std::path::PathBuf;

pub static TEST_DOMAIN: &str = "fixtures/test_domain";
pub static TEST_LOCAL_PARQUET: &str = ".quilt/packages/12201234.parquet";
pub static TEST_LOCAL_PARQUET_CHECKSUMED: &str = ".quilt/packages/checksumed.parquet";
pub static TEST_LOCAL_JSONL: &str =
    ".quilt/packages/0428ab8c8b0fe83d9e57fb6b26ff190173caad00ed7aeb683ce26cc4b56ea4bb";

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
        (TestFile::ParquetChecksummed, TEST_LOCAL_PARQUET_CHECKSUMED),
        (TestFile::Json, TEST_LOCAL_JSONL),
        (TestFile::Domain, ""),
    ]);

    let cwd = std::env::current_dir().unwrap();
    let domain = cwd.join(TEST_DOMAIN);
    domain.join(files[&key])
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
