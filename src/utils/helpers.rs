use super::*;
use std::collections::HashMap;

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

pub fn local_uri(key: TestFile) -> String {
    let files: HashMap<TestFile, &str> = HashMap::from([
        (TestFile::Parquet, TEST_LOCAL_PARQUET),
        (TestFile::Json, TEST_LOCAL_JSONL),
        (TestFile::Domain, ""),
    ]);

    let cwd = std::env::current_dir().unwrap();
    let domain = cwd.join(TEST_DOMAIN);
    let path = domain.join(files[&key]);
    let path_string = path.to_string_lossy();
    format!("file://{}", path_string)
}

pub fn local_uri_domain() -> String {
    local_uri(TestFile::Domain)
}

pub fn local_uri_parquet() -> String {
    local_uri(TestFile::Parquet)
}

pub fn local_uri_json() -> String {
    local_uri(TestFile::Json)
}

pub fn current_domain() -> String {
    format!(
        "file://{}/tests/test_domain/.quilt",
        std::env::current_dir().unwrap().to_string_lossy()
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_uri_domain() {
        let expected = current_domain();
        let actual = format!("{}.quilt", local_uri_domain());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_local_uri_parquet() {
        let expected = format!("{}/packages/12201234.parquet", current_domain());
        assert_eq!(local_uri_parquet(), expected);
    }

    #[test]
    fn test_local_uri_json() {
        let expected = format!(
            "{}/packages/5f1b1e4928dbb5d700cfd37ed5f5180134d1ad93a0a700f17e43275654c262f4",
            current_domain()
        );
        assert_eq!(local_uri_json(), expected);
    }
}
