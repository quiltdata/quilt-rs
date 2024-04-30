mod manifest;
mod package;
mod s3;
pub mod tag;

// FIXME: add TagUri and ManifestUri

pub use manifest::RemoteManifest;
pub use package::Namespace;
pub use package::RevisionPointer;
pub use package::S3PackageUri;
pub use s3::S3Uri;
pub use tag::TagUri;
