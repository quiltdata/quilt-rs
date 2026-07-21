//!
//! Namespace containing various URIs.
//! Most of them you can convert one to another.

// Test fns return `Res` and end in `Ok(())` so the body can use `?`; that trips
// `unnecessary_wraps` in tests only. Enforce it in production, allow under test.
#![cfg_attr(test, allow(clippy::unnecessary_wraps))]

pub mod error;
mod host;
mod manifest;
mod object;
mod package;
pub mod paths;
mod s3;
mod tag;

pub use error::UriError;
pub use host::Host;
pub use manifest::ManifestUri;
pub use object::ObjectUri;
pub use package::Namespace;
pub use package::RevisionPointer;
pub use package::S3PackageHandle;
pub use package::S3PackageUri;
pub use s3::S3Uri;
pub use tag::Seconds;
pub use tag::Tag;
pub use tag::TagUri;
