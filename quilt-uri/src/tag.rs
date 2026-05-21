use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
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
            Err(UriError::Tag(format!(
                "tag must be \"latest\" or a Unix timestamp in seconds; got: {s}"
            )))
        }
    }
}

impl From<Tag> for String {
    fn from(tag: Tag) -> String {
        tag.to_string()
    }
}

impl TryFrom<String> for Tag {
    type Error = UriError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
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
    handle: S3PackageHandle,
    tag: Tag,
}

impl TagUri {
    pub fn new(uri: impl Into<S3PackageHandle>, tag: Tag) -> Self {
        TagUri {
            handle: uri.into(),
            tag,
        }
    }

    /// Creates `TagURI` for the latest revision of the package
    pub fn latest(uri: impl Into<S3PackageHandle>) -> Self {
        TagUri::new(uri, Tag::Latest)
    }

    /// Creates `TagURI` for the revision of the package.
    pub fn timestamp(uri: impl Into<S3PackageHandle>, seconds: Seconds) -> Self {
        TagUri::new(uri, Tag::Timestamp(seconds))
    }
}

impl From<TagUri> for S3PackageHandle {
    fn from(uri: TagUri) -> S3PackageHandle {
        uri.handle
    }
}

impl From<TagUri> for S3Uri {
    fn from(uri: TagUri) -> S3Uri {
        let key = paths::tag_key(&uri.handle.namespace, &uri.tag.to_string());
        S3Uri {
            bucket: uri.handle.bucket,
            key,
            version: None,
        }
    }
}

impl From<&TagUri> for S3Uri {
    fn from(uri: &TagUri) -> S3Uri {
        let key = paths::tag_key(&uri.handle.namespace, &uri.tag.to_string());
        S3Uri {
            bucket: uri.handle.bucket.clone(),
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
        assert_eq!(tag, Tag::Timestamp(Seconds(1_697_916_638)));
    }

    #[test]
    fn test_tag_from_str_invalid() {
        let result: Result<Tag, _> = "invalid-tag".parse();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("tag must be \"latest\" or a Unix timestamp")
        );
    }

    #[test]
    fn test_tag_serde_round_trip_latest() {
        let json = serde_json::to_string(&Tag::Latest).unwrap();
        assert_eq!(json, "\"latest\"");
        let parsed: Tag = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Tag::Latest);
    }

    #[test]
    fn test_tag_serde_round_trip_timestamp() {
        let tag = Tag::Timestamp(Seconds(1_697_916_638));
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, "\"1697916638\"");
        let parsed: Tag = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tag);
    }

    #[test]
    fn test_tag_display() {
        assert_eq!(Tag::Latest.to_string(), "latest");
        assert_eq!(
            Tag::Timestamp(Seconds(1_697_916_638)).to_string(),
            "1697916638"
        );
    }
}
