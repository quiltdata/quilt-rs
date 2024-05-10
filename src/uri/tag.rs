use std::fmt;

use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3PackageHandle;
use crate::uri::S3Uri;

/// In theory tag can be any string
/// But in practice we only use timestamps and "latest"
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Timestamp(i64),
    Latest,
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tag::Timestamp(t) => write!(f, "{}", t),
            Tag::Latest => write!(f, "latest"),
        }
    }
}

/// Tag URI is an URI for tagged revisions of packages
/// We have directories for named packages (or just "packages"). These directories contain
/// revisions for these named packages
/// Each revision is tagged by timestamp or "latest" and contain reference to the actual manifest
/// by hash.
/// So, it is an URI for the file that contains link to immutable unnamed manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagUri {
    bucket: String,
    namespace: Namespace,
    tag: Tag,
}

impl TagUri {
    pub fn new(bucket: String, namespace: Namespace, tag: Tag) -> Self {
        TagUri {
            bucket,
            namespace,
            tag,
        }
    }

    /// Creates TagURI for the latest revision of the package
    pub fn latest(uri: S3PackageHandle) -> Self {
        TagUri::new(uri.bucket, uri.namespace, Tag::Latest)
    }

    /// Creates TagURI for the revision of the package
    pub fn timestamp(manifest_uri: ManifestUri, datetime: chrono::DateTime<chrono::Utc>) -> Self {
        TagUri {
            bucket: manifest_uri.bucket,
            namespace: manifest_uri.namespace,
            tag: Tag::Timestamp(datetime.timestamp()),
        }
    }
}

impl From<TagUri> for S3Uri {
    fn from(uri: TagUri) -> S3Uri {
        let key = paths::tag_key(&uri.namespace, &uri.tag.to_string());
        S3Uri {
            bucket: uri.bucket,
            key,
            version: None,
        }
    }
}
