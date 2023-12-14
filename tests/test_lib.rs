use quilt_rs::browse_remote_package;
use quilt_rs::LocalDomain;
use quilt_rs::Manifest;
use quilt_rs::S3PackageURI;

#[tokio::test]
async fn test_browse_remote_package() {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let test_uri_string = "quilt+s3://quilt-t4-staging#package=test/sync&path=README.md";
    let test_uri = S3PackageURI::try_from(test_uri_string).expect("Failed to parse URI");
    let manifest: Manifest = browse_remote_package(local_domain.into(), test_uri)
        .await
        .expect("Failed to browse remote package");
    println!("manifest: {:#?}", manifest);
    assert!(manifest.rows.len() > 0);
}