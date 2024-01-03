use quilt_rs::*;
use utils::local_uri_parquet;

pub async fn make_manifest(cl: &Client, up: UPath) -> Manifest4 {
    let dom = Domain::new(cl, up.clone());
    let ns = dom.get("manual/test").await.unwrap();
    manifest
}
 
#[tokio::test]
async fn test_quilt4_manifest() {
    let path_name = local_uri_parquet();
    let up = UPath::parse(&path_name).unwrap();
    let cl = Client::new();
    let manifest = make_manifest(&cl, up).await;
    // serialize manifest to Json using Serde
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(json.contains("test"));
}
