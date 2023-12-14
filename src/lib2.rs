pub use quilt_rs::LocalDomain;
pub use quilt_rs::Manifest;
pub use quilt_rs::ManifestHeader;
pub use quilt_rs::ManifestRow;
pub use quilt_rs::RemoteManifest;
pub use quilt_rs::S3PackageURI;

pub use quilt_rs::browse_remote_package;
pub use quilt_rs::browse_remote_manifest;
pub use quilt_rs::list_installed_packages;

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