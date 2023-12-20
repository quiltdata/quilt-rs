use quilt_rs::*;
use utils::local_uri_parquet;

pub async fn make_manifest(path_name: String) -> Manifest4 {
    let up = UPath::new(path_name);
    let cl = Client::new();
    let dom = Domain::new(cl.clone(), up.clone()).await;
    let nam = Namespace::new(dom, up.clone()).await;
    let tab = Table::new(Some(up.clone())).read4().unwrap();
    let manifest = Manifest4::new(
        nam.clone(),
        tab.clone(),
        Some(up.clone())
    ).await;
    manifest
}
 
#[tokio::test]
async fn test_quilt4_manifest() {
    let manifest = make_manifest(local_uri_parquet()).await;
    // serialize manifest to Json using Serde
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(json.contains("test"));
}
