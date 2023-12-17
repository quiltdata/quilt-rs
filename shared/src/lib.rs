use std::collections::HashMap;

pub static TEST_URI_STRING: &str = "quilt+s3://quilt-example#package=akarve/test_dest";

pub static TEST_DOMAIN: &str = "tests/test_domain";

pub static TEST_LOCAL_PARQUET: &str =  "./tests/test_domain/packages/12201234.parquet";

pub static TEST_LOCAL_JSONL: &str = "./tests/test_domain/.quilt/packages/5f1b1e4928dbb5d700cfd37ed5f5180134d1ad93a0a700f17e43275654c262f4";

pub static TEST_S3_URI: &str = "s3://quilt-example/akarve/test_dest/README.md";

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum TestFile {
    Parquet,
    Json,
    Domain,
}

pub fn local_domain_uri(key: TestFile) -> String {
    let files: HashMap<TestFile,&str> = HashMap::from ([
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_domain_uri_parquet() {
        let expected = format!("file://{}/tests/test_domain/packages/12201234.parquet", std::env::current_dir().unwrap().to_string_lossy());
        assert_eq!(local_domain_uri(TestFile::Parquet), expected);
    }

    #[test]
    fn test_local_domain_uri_json() {
        let expected = format!("file://{}/tests/test_domain/.quilt/packages/5f1b1e4928dbb5d700cfd37ed5f5180134d1ad93a0a700f17e43275654c262f4", std::env::current_dir().unwrap().to_string_lossy());
        assert_eq!(local_domain_uri(TestFile::Json), expected);
    }

    #[test]
    fn test_local_domain_uri_domain() {
        let expected = format!("file://{}/tests/test_domain", std::env::current_dir().unwrap().to_string_lossy());
        assert_eq!(local_domain_uri(TestFile::Domain), expected);
    }
}
