use quilt_rs::manifest_from_uri;
use quilt_rs::installed_packages;
use quilt_rs::Client;

extern crate shared;

static URI_STRING: &str = shared::TEST_URI_STRING;

#[tokio::test]
async fn test_browse_remote_package() {
    let manifest = manifest_from_uri(URI_STRING.to_string()).await.unwrap();
    assert!(manifest.rows.len() > 0);
    let installed = installed_packages().await.unwrap();
    println!("installed: {:#?}", installed)
    //assert!(installed.len() > 0);
}

#[tokio::test]
async fn test_manifest3_from_uri() {
    // Arrange
    let client = Client::new().await;

    // Act
    let result = client.manifest3_from_uri(URI_STRING.to_string()).await;

    // Assert
    assert!(result.is_ok());
    let manifest = result.unwrap();
    assert!(manifest.rows.len() > 0);
}
