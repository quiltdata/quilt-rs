mod package;
mod s3;

// FIXME: add TagUri and ManifestUri

pub use package::Namespace;
pub use package::RevisionPointer;
pub use package::S3PackageUri;
pub use s3::S3Uri;
