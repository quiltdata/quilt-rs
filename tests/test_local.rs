use quilt_rs::LocalDomain;
use utils::{TEST_BUCKET, TEST_PACKAGE, TEST_HASH};
use temp_dir::TempDir;

#[tokio::test]
async fn test_local_manifest() {
    let temp_folder = TempDir::new().unwrap();
    let loc = LocalDomain::new(temp_folder.path().to_path_buf());
    let remote_manifest = quilt_rs::RemoteManifest {
        bucket: TEST_BUCKET.to_string(),
        namespace: TEST_PACKAGE.to_string(),
        hash: TEST_HASH.to_string(),
    };
    println!("remote_manifest: {:?}", remote_manifest);

    let result = loc.install_package(&remote_manifest).await;
    println!("result: {:?}", result);

    assert!(result.is_ok());
    let manifest = result.unwrap();
    assert_eq!(manifest.namespace, TEST_PACKAGE);
}
