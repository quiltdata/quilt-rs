use aws_sdk_s3::Error; // Import the Error type from the aws_sdk_s3 crate
mod api;
mod quilt;
mod s3_utils;

pub use api::LocalDomain;
pub use api::Manifest;
pub use api::ManifestHeader;
pub use quilt::ManifestRow;
pub use quilt::InstalledPackage;
pub use api::RemoteManifest;
pub use api::S3PackageURI;

pub use api::browse_remote_package;
pub use api::browse_remote_manifest;
pub use api::list_installed_packages;

pub async fn manifest_from_uri(uri_string: String) -> Result<Manifest, Error> {
    let path_buf = std::env::current_dir().unwrap();
    let local_domain = LocalDomain::new(path_buf);
    let uri = S3PackageURI::try_from(uri_string.as_str()).expect("Failed to parse URI");
    let manifest: Manifest = browse_remote_package(local_domain.into(), uri)
        .await
        .expect("Failed to browse remote package");
    println!("manifest: {:#?}", manifest);
    assert!(manifest.rows.len() > 0);
    manifest.rows.len();
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manifest_from_uri() {
        let uri = "quilt+s3://quilt-example#package=akarve/test_dest".to_string();
        let manifest = manifest_from_uri(uri).await;
        assert!(manifest.is_ok());
        assert!(manifest.unwrap().rows.len() > 0);
    }
}