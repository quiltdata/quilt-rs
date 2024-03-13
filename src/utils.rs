pub mod helpers;
// pub mod parquet_object_store;

pub static TEST_REGION: &str = "us-east=1";
pub static TEST_BUCKET: &str = "quilt-example";
pub static TEST_PACKAGE: &str = "akarve/test_dest";
pub static TEST_FILE: &str = "README.md";
pub static TEST_HASH: &str = "6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22";
pub static TEST_URI_STRING: &str = "quilt+s3://quilt-example#package=akarve/test_dest";
pub static TEST_S3_URI: &str = "s3://quilt-example/akarve/test_dest/README.md";

pub static TEST_DOMAIN: &str = "tests/test_domain";
pub static TEST_LOCAL_PARQUET: &str = ".quilt/packages/12201234.parquet";
pub static TEST_LOCAL_JSONL: &str =
    ".quilt/packages/5f1b1e4928dbb5d700cfd37ed5f5180134d1ad93a0a700f17e43275654c262f4";

pub use helpers::local_uri_domain;
pub use helpers::local_uri_json;
pub use helpers::local_uri_parquet;

pub use helpers::remote_quilt_uri;
pub use helpers::remote_s3_uri;
