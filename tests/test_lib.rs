use quilt_rs::manifest_from_uri;
use quilt_rs::installed_packages;

#[tokio::test]
async fn test_browse_remote_package() {
    let test_uri_string = "quilt+s3://quilt-example#package=akarve/test_dest&path=README.md";
    let manifest = manifest_from_uri(test_uri_string.to_string()).await.unwrap();
    assert!(manifest.rows.len() > 0);
    let installed = installed_packages().await.unwrap();
    assert!(installed.len() > 0);
}