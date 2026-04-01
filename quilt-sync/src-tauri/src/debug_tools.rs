use crate::quilt;
use crate::Error;

pub fn try_remote_origin_host(uri: &quilt::uri::ManifestUri) -> Result<quilt::uri::Host, Error> {
    uri.origin.clone().ok_or(Error::MissingOrigin)
}

pub fn try_remote_package_origin_host(
    remote: &quilt::lineage::RemotePackage,
) -> Result<quilt::uri::Host, Error> {
    remote.origin.clone().ok_or(Error::MissingOrigin)
}

/// Resolve the package URI and origin host from lineage.
///
/// For remote packages, derives both from the `RemotePackage`.
/// For local-only packages, builds a placeholder URI from the namespace.
pub fn resolve_uri_and_host(
    remote: Option<&quilt::lineage::RemotePackage>,
    namespace: &quilt::uri::Namespace,
) -> (quilt::uri::S3PackageUri, Option<quilt::uri::Host>) {
    match remote {
        Some(remote) => {
            let uri = remote.to_s3_uri();
            let host = try_remote_package_origin_host(remote).ok();
            (uri, host)
        }
        // TODO: local-only packages only need `namespace` here, but pages
        // and ViewEntry thread a full S3PackageUri everywhere.
        // Refactor view models to use Namespace directly instead of a fake URI.
        None => {
            let uri = quilt::uri::S3PackageUri {
                catalog: None,
                bucket: String::new(),
                namespace: namespace.clone(),
                revision: quilt::uri::RevisionPointer::Tag(quilt::uri::LATEST_TAG.to_string()),
                path: None,
            };
            (uri, None)
        }
    }
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
        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "bucket".to_string(),
            origin: None,
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
        // Test with explicit origin
        let manifest_with_origin = quilt::uri::ManifestUri {
            bucket: "bucket".to_string(),
            origin: Some("custom.quilt.org".parse()?),
            hash: "abcdef".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert_eq!(
            try_remote_origin_host(&manifest_with_origin)?.to_string(),
            "custom.quilt.org"
        );

        // Test with no origin returns MissingOrigin error
        let manifest_without_origin = quilt::uri::ManifestUri {
            bucket: "bucket".to_string(),
            origin: None,
            hash: "abcdef".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert!(matches!(
            try_remote_origin_host(&manifest_without_origin),
            Err(Error::MissingOrigin)
        ));

        Ok(())
    }
}
