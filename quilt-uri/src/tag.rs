use std::fmt;
use std::str::FromStr;

use crate::ManifestUri;
use crate::Namespace;
use crate::S3PackageHandle;
use crate::S3Uri;
use crate::UriError;
use crate::paths;

/// Unix timestamp in seconds since the epoch.
///
/// Wrapping the raw `i64` prevents unit confusion at API boundaries
/// (e.g. accidentally passing milliseconds from `SystemTime::elapsed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Seconds(pub i64);

impl fmt::Display for Seconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Seconds {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<i64>().map(Seconds)
    }
}

/// In theory tag can be any string
/// But in practice we only use timestamps and "latest"
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Timestamp(Seconds),
    Latest,
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tag::Timestamp(t) => write!(f, "{t}"),
            Tag::Latest => write!(f, "latest"),
        }
    }
}

impl FromStr for Tag {
    type Err = UriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "latest" {
            Ok(Tag::Latest)
        } else if let Ok(seconds) = s.parse::<Seconds>() {
            Ok(Tag::Timestamp(seconds))
        } else {
            Err(UriError::Tag(format!("Unsupported tag format: {s}")))
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

    /// Creates TagURI for the revision of the package.
    pub fn timestamp(manifest_uri: ManifestUri, seconds: Seconds) -> Self {
        TagUri {
            bucket: manifest_uri.bucket,
            namespace: manifest_uri.namespace,
            tag: Tag::Timestamp(seconds),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_from_str_latest() {
        let tag: Tag = "latest".parse().unwrap();
        assert_eq!(tag, Tag::Latest);
    }

    #[test]
    fn test_tag_from_str_timestamp() {
        let tag: Tag = "1697916638".parse().unwrap();
        assert_eq!(tag, Tag::Timestamp(Seconds(1697916638)));
    }

    #[test]
    fn test_tag_from_str_invalid() {
        let result: Result<Tag, _> = "invalid-tag".parse();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported tag format")
        );
    }

    #[test]
    fn test_tag_display() {
        assert_eq!(Tag::Latest.to_string(), "latest");
        assert_eq!(
            Tag::Timestamp(Seconds(1697916638)).to_string(),
            "1697916638"
        );
    }
}
