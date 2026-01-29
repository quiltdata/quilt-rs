use crate::quilt;
use crate::Error;

pub fn try_remote_origin_host(
    uri: &quilt::uri::ManifestUriParquet,
) -> Result<quilt::uri::Host, Error> {
    Ok(uri.catalog.clone().unwrap_or("open.quilt.bio".parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Error;

    #[test]
    fn test_stringify_s3_package_uri() -> Result<(), Error> {
        let ref_uri = "quilt+s3://bucket#package=foo/bar&path=root/readme.md";
        let uri = quilt::uri::S3PackageUri::try_from(ref_uri)?;
        assert_eq!(uri.to_string(), ref_uri.to_string());
        Ok(())
    }

    #[test]
    fn test_stringify_remote_manifest() {
        let remote_manifest = quilt::uri::ManifestUriParquet {
            bucket: "bucket".to_string(),
            catalog: None,
            hash: "abcdef".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert_eq!(
            remote_manifest.to_string(),
            "quilt+s3://bucket#package=foo/bar@abcdef"
        )
    }

    #[test]
    fn test_try_remote_origin_host() -> Result<(), Error> {
        // Test with explicit catalog
        let manifest_with_catalog = quilt::uri::ManifestUriParquet {
            bucket: "bucket".to_string(),
            catalog: Some("custom.quilt.org".parse()?),
            hash: "abcdef".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert_eq!(
            try_remote_origin_host(&manifest_with_catalog)?.to_string(),
            "custom.quilt.org"
        );

        // Test with default catalog
        let manifest_without_catalog = quilt::uri::ManifestUriParquet {
            bucket: "bucket".to_string(),
            catalog: None,
            hash: "abcdef".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert_eq!(
            try_remote_origin_host(&manifest_without_catalog)?.to_string(),
            "open.quilt.bio"
        );

        Ok(())
    }
}
