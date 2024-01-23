use quilt_rs::*;
use utils::local_uri_parquet;

#[tokio::test]
async fn test_quilt4_manifest() {
    let path_name = local_uri_parquet();
    let up = UPath::parse(&path_name).unwrap();
    let temp_dir = temp_testdir::TempDir::default();
    let cl = Client::new(LocalDomain::new(temp_dir.to_path_buf()));
    let dom = Domain::new(&cl, up);
    let manifest = dom.get_latest("manual/test").await;
    let result = manifest.to_string();
    assert!(result.contains("test"));
}
