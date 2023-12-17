use quilt_rs::manifest_from_uri;

#[tokio::main]
async fn main() {
    let default_uri = "quilt+s3://quilt-example#package=akarve/test_dest".to_string();
    // TODO: replace with shared::TEST_URI_STRING
    let args: Vec<String> = std::env::args().collect();
    let uri = if args.len() > 1 { args[1].clone() } else { default_uri };
    let manifest = manifest_from_uri(uri).await;
    println!("manifest: {:#?}", manifest);
}
