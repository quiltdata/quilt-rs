use quilt_rs::*;
use utils::local_uri_parquet;

#[tokio::test]
async fn test_quilt4_manifest() {
    let path_name = local_uri_parquet();
    let up = UPath::parse(&path_name).unwrap();
    let cl = Client::new();
    let dom = Domain::new(&cl, up);
    let manifest = dom.get_latest("manual/test").await.unwrap();
    let result = manifest.to_string();
    assert!(result.contains("test"));
}
