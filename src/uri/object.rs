use std::fmt;
use std::path::PathBuf;

use crate::uri::Namespace;
use crate::uri::S3Uri;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub path: PathBuf,
    pub version: Option<String>,
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
