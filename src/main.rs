use quilt_rs::{install_temporarily, quilt::{package_s3_prefix, s3::S3Uri}};
use tracing::info;
use quilt_rs::utils::{TEST_BUCKET, TEST_PACKAGE, TEST_HASH};

#[tokio::main]
async fn main() {
    /*
    // let args: Vec<String> = std::env::args().collect();
    // let uri_string = if args.len() > 1 { &args[1] } else { &default_uri };
    let manifest = install_temporarily(
        TEST_BUCKET,
        TEST_PACKAGE,
        TEST_HASH,
    ).await;
    info!("manifest: {:#?}", manifest);
    */

    let uri = S3Uri {
        bucket: "data-yaml-spec-tests".into(),
        key: "scale/10u/".into(),
        version: None,
    };
    package_s3_prefix("test/test", &uri).await;
}
