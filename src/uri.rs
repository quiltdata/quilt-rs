//!
//! Namespace containing various URIs.
//! Most of them you can convert one to another.

mod host;
mod manifest;
mod object;
mod package;
mod s3;
mod tag;

pub use host::Host;
pub use manifest::ManifestUri;
pub use manifest::ManifestUriLegacy;
pub use object::ObjectUri;
pub use package::Namespace;
pub use package::RevisionPointer;
pub use package::S3PackageHandle;
pub use package::S3PackageUri;
pub use s3::S3Uri;
pub use tag::TagUri;
