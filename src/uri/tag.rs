use crate::paths;
use crate::quilt::manifest_handle::RemoteManifest;
use crate::quilt::Namespace;
use crate::uri::S3Uri;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Timestamp(String),
    Latest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub tag: Tag,
}

impl TagUri {
    pub fn new(bucket: String, namespace: Namespace, tag: Tag) -> Self {
        TagUri {
            bucket,
            namespace,
            tag,
        }
    }

    pub fn latest(remote_manifest: &RemoteManifest) -> Self {
        let RemoteManifest {
            bucket, namespace, ..
        } = remote_manifest;
        TagUri::new(bucket.clone(), namespace.clone(), Tag::Latest)
    }

    pub fn timestamp(
        remote_manifest: &RemoteManifest,
        datetime: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let RemoteManifest {
            bucket, namespace, ..
        } = remote_manifest;
        TagUri {
            bucket: bucket.clone(),
            namespace: namespace.clone(),
            tag: Tag::Timestamp(datetime.timestamp().to_string()),
        }
    }
}

impl From<TagUri> for S3Uri {
    fn from(uri: TagUri) -> S3Uri {
        let tag = match uri.tag {
            Tag::Timestamp(timestamp) => timestamp,
            Tag::Latest => "latest".to_string(),
        };
        S3Uri {
            bucket: uri.bucket,
            key: paths::tag_key(&uri.namespace, &tag),
            version: None,
        }
    }
}
