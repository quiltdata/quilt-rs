//! Re-exports the `quilt-uri` crate so existing `quilt_rs::uri::...`
//! call sites keep compiling unchanged.

pub use quilt_uri::Host;
pub use quilt_uri::ManifestUri;
pub use quilt_uri::Namespace;
pub use quilt_uri::ObjectUri;
pub use quilt_uri::RevisionPointer;
pub use quilt_uri::S3PackageHandle;
pub use quilt_uri::S3PackageUri;
pub use quilt_uri::S3Uri;
pub use quilt_uri::Seconds;
pub use quilt_uri::Tag;
pub use quilt_uri::TagUri;
pub use quilt_uri::UriError;
pub use quilt_uri::LATEST_TAG;
