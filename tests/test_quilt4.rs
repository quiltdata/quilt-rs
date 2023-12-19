use quilt_rs::*;
 
#[tokio::test]
async fn test_quilt4_manifest() {
    let up = UPath::new("test".to_string());
    let cl = Client::new().await;
    let dom = Domain::new(cl.clone(), up.clone()).await;
    let nam = Namespace::new(dom, up.clone()).await;
    let tab = Table::new(Some(up.clone())).await;
    let manifest = Manifest4::new(
        nam.clone(),
        tab.clone(),
        Some(up.clone())
    ).await;
    assert!(manifest.to_string().starts_with("Manifest4(UPath(test)"));
}

