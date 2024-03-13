use quilt_rs::install_temporarily;
use quilt_rs::utils::{TEST_BUCKET, TEST_HASH, TEST_PACKAGE};
use tracing::info;

#[tokio::main]
async fn main() {
    // let args: Vec<String> = std::env::args().collect();
    // let uri_string = if args.len() > 1 { &args[1] } else { &default_uri };
    let manifest = install_temporarily(TEST_BUCKET, TEST_PACKAGE, TEST_HASH).await;
    info!("manifest: {:#?}", manifest);
}
