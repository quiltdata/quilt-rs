use quilt_rs::manifest_from_uri;
use tracing::info;

#[tokio::main]
async fn main() {
    let default_uri = "quilt+s3://quilt-example#package=akarve/test_dest";
    // TODO: replace with utils::TEST_URI_STRING
    let args: Vec<String> = std::env::args().collect();
    let uri = if args.len() > 1 { &args[1] } else { default_uri };
    let manifest = manifest_from_uri(uri).await;
    info!("manifest: {:#?}", manifest);
}
