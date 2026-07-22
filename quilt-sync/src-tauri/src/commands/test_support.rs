//! Shared fixtures for command tests.

use crate::quilt;

/// Stringify the catalog host of a typed package URI, if set.
/// Used by tests to verify host propagation without poking at
/// `Option<&Host>` chains inline.
pub(crate) fn catalog_host(uri: Option<&quilt_uri::S3PackageUri>) -> Option<String> {
    uri.and_then(|u| u.catalog.as_ref())
        .map(std::string::ToString::to_string)
}

/// Helper: create a `quilt::InstalledPackage` with a given namespace.
pub(crate) fn make_installed_package(
    namespace: impl Into<quilt_uri::Namespace>,
) -> quilt::InstalledPackage {
    quilt::LocalDomain::new(std::path::PathBuf::new())
        .create_installed_package(namespace.into())
        .expect("Failed to create installed package")
}

/// Helper: create a `ManifestUri` with origin for a given namespace.
pub(crate) fn make_manifest_uri(namespace: &str) -> quilt_uri::ManifestUri {
    quilt_uri::ManifestUri {
        origin: Some("test.quilt.dev".parse().unwrap()),
        bucket: "test".to_string(),
        namespace: namespace.try_into().unwrap(),
        hash: "abcdef".to_string(),
    }
}

/// Helper: create a `ManifestUri` **without** origin (triggers error state).
pub(crate) fn make_manifest_uri_no_origin(namespace: &str) -> quilt_uri::ManifestUri {
    quilt_uri::ManifestUri {
        origin: None,
        bucket: "test".to_string(),
        namespace: namespace.try_into().unwrap(),
        hash: "abcdef".to_string(),
    }
}
