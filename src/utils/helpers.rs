use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum TestFile {
    Parquet,
    Json,
    Domain,
}

pub fn remote_quilt_uri() -> String {
    format!(
        "quilt+s3://{}#package={}&path={}",
        TEST_BUCKET, TEST_PACKAGE, TEST_FILE
    )
}

pub fn remote_s3_uri() -> String {
    format!("s3://{}/{}", TEST_BUCKET, TEST_PACKAGE)
}

pub fn local_uri(key: TestFile) -> PathBuf {
    let files: HashMap<TestFile, &str> = HashMap::from([
        (TestFile::Parquet, TEST_LOCAL_PARQUET),
        (TestFile::Json, TEST_LOCAL_JSONL),
        (TestFile::Domain, ""),
    ]);

    let cwd = std::env::current_dir().unwrap();
    let domain = cwd.join(TEST_DOMAIN);
    domain.join(files[&key])
}

pub fn local_uri_domain() -> PathBuf {
    local_uri(TestFile::Domain)
}

pub fn local_uri_parquet() -> PathBuf {
    local_uri(TestFile::Parquet)
}

pub fn local_uri_json() -> PathBuf {
    local_uri(TestFile::Json)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn current_domain() -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("./tests/test_domain/.quilt")
    }

    #[test]
    fn test_local_uri_domain() {
        let expected = current_domain();
        let actual = local_uri_domain().join(".quilt");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_local_uri_parquet() {
        let expected = current_domain().join("./packages/12201234.parquet");
        assert_eq!(local_uri_parquet(), expected);
    }

    #[test]
    fn test_local_uri_json() {
        let expected = current_domain()
            .join("./packages/0428ab8c8b0fe83d9e57fb6b26ff190173caad00ed7aeb683ce26cc4b56ea4bb");
        assert_eq!(local_uri_json(), expected);
    }
}
