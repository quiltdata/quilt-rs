use std::fmt;
use std::path::PathBuf;

use crate::uri::Namespace;
use crate::uri::S3PackageHandle;
use crate::uri::S3Uri;

/// Object URI is an URI for objects in packages.
/// In packages they are stored as logical keys.
/// Physically they can be stored anywhere, but their place is this URI.
///
/// It knows where to put new objects and how to convert itself to S3URI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectUri {
    bucket: String,
    namespace: Namespace,
    path: PathBuf,
    version: Option<String>,
}

impl ObjectUri {
    pub fn new(package_handle: S3PackageHandle, path: PathBuf) -> Self {
        ObjectUri {
            bucket: package_handle.bucket,
            namespace: package_handle.namespace,
            path,
            version: None,
        }
    }
}

impl From<ObjectUri> for S3Uri {
    fn from(uri: ObjectUri) -> S3Uri {
        S3Uri {
            bucket: uri.bucket.to_string(),
            key: format!("{}/{}", uri.namespace, uri.path.display()),
            version: uri.version,
        }
    }
}

impl From<&ObjectUri> for S3Uri {
    fn from(uri: &ObjectUri) -> S3Uri {
        S3Uri::from(uri.clone())
    }
}

impl fmt::Display for ObjectUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", S3Uri::from(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_uri_display() {
        let uri = ObjectUri {
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            path: PathBuf::from("data/file.txt"),
            version: Some("final".to_string()),
        };
        assert_eq!(uri.to_string(), "s3://test-bucket/foo/bar/data/file.txt?versionId=final");
    }
}
