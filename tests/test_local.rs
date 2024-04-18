use tracing::log;

use quilt_rs::install_temporarily;
use quilt_rs::utils::TEST_BUCKET;
use quilt_rs::utils::TEST_HASH;
use quilt_rs::utils::TEST_PACKAGE;

#[tokio::test]
async fn test_local_manifest() {
    let result = install_temporarily(TEST_BUCKET, TEST_PACKAGE, TEST_HASH).await;
    log::debug!("result: {:?}", result);

    assert!(result.is_ok());
    let manifest = result.unwrap();
    assert_eq!(manifest.namespace, TEST_PACKAGE);
}
