use quilt_rs::install_temporarily;
use crate::utils::{TEST_BUCKET, TEST_PACKAGE, TEST_HASH};

#[tokio::test]
async fn test_local_manifest() {
  
    let result = install_temporarily(
        TEST_BUCKET,
        TEST_PACKAGE,
        TEST_HASH,
    ).await;
    println!("result: {:?}", result);

    assert!(result.is_ok());
    let manifest = result.unwrap();
    assert_eq!(manifest.namespace, TEST_PACKAGE);
}
