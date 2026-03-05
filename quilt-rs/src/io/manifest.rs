//! Contains utility functions to work with manifests.

use std::marker::Unpin;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncReadExt;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tracing::log;
use url::Url;

use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
#[cfg(test)]
use crate::manifest::MetadataSchema;
use crate::manifest::TopHasher;
#[cfg(test)]
use crate::manifest::Workflow;
#[cfg(test)]
use crate::manifest::WorkflowId;
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::ObjectUri;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageHandle;
use crate::uri::S3PackageUri;
use crate::uri::Tag;
use crate::uri::TagUri;
use crate::Error;
use crate::Res;

async fn bytestream_to_string(bytestream: ByteStream) -> Res<String> {
    let mut reader = bytestream.into_async_read();
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents).await?;
    String::from_utf8(contents).map_err(|err| Error::Utf8(err.utf8_error()))
}

/// Upload manifest from the local path
/// We don't care about checksum of the resulted file.
async fn upload_from(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_path: &PathBuf,
    manifest_uri: &ManifestUri,
) -> Res {
    // TODO: FAIL if the manifest with this hash already exists?
    let body = storage.read_byte_stream(manifest_path).await?;
    log::info!("Writing remote manifest to {manifest_uri:?}");
    remote
        .put_object(&manifest_uri.origin, &manifest_uri.clone().into(), body)
        .await
}

/// Upload JSONL manifest to remote.
/// We don't care about checksum of the resulted file.
pub async fn upload_manifest(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    path: &PathBuf,
) -> Res {
    // Upload the JSONL manifest to the remote, don't tag it yet
    upload_from(storage, remote, path, manifest_uri).await?;
    log::info!("JSONL manifest uploaded");

    log::info!("Uploaded remote manifest: {manifest_uri:?}");
    Ok(())
}

/// Upload file containing hash of the manifest
/// "tagged" by timestamp.
pub async fn tag_timestamp(
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Res {
    // Tag the new commit.
    // If {self.commit.tag} does not already exist at
    // {self.remote}/.quilt/named_packages/{self.namespace},
    // create it with the value of {self.commit.hash}
    // TODO: Otherwise try again with the current timestamp as the tag
    // (e.g., try five times with exponential backoff, then Error)
    let tag_timestamp = TagUri::timestamp(manifest_uri.clone(), timestamp);
    upload_tag(remote, manifest_uri, tag_timestamp).await
}

/// Upload file containing hash of the manifest
/// "tagged" as "latest".
pub async fn tag_latest(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res {
    let tag_latest = TagUri::latest(manifest_uri.clone().into());
    upload_tag(remote, manifest_uri, tag_latest).await
}

async fn upload_tag(remote: &impl Remote, manifest_uri: &ManifestUri, tag_uri: TagUri) -> Res {
    remote
        .put_object(
            &manifest_uri.origin,
            &tag_uri.into(),
            manifest_uri.hash.as_bytes().to_vec(),
        )
        .await
}

/// Downloads the tagged package (latest or timestamp)
/// and returns its content: hash of the tagged package revision.
/// Then creates `ManifestUri`.
pub async fn resolve_tag(
    remote: &impl Remote,
    host: &Option<Host>,
    uri: &S3PackageHandle,
    tag: Tag,
) -> Res<ManifestUri> {
    let tag_uri = TagUri::new(uri.bucket.clone(), uri.namespace.clone(), tag);
    let stream = remote.get_object_stream(host, &tag_uri.into()).await?;
    let hash = bytestream_to_string(stream.body).await?;
    let S3PackageHandle { bucket, namespace } = uri.to_owned();
    let origin = host.to_owned();
    Ok(ManifestUri {
        hash,
        bucket,
        namespace,
        origin,
    })
}

/// `ManifestUri` should always have `hash`.
/// But `S3PackageUri` can be tagged (e.g. "latest" or timestamp).
/// So, we need to download the tag and find out what the `hash` is
async fn resolve_top_hash(
    remote: &impl Remote,
    host: &Option<Host>,
    uri: &S3PackageUri,
) -> Res<String> {
    match &uri.revision {
        RevisionPointer::Hash(top_hash) => Ok(top_hash.clone()),
        RevisionPointer::Tag(tag_str) => {
            Ok(resolve_tag(remote, host, &uri.into(), tag_str.parse()?)
                .await?
                .hash)
        }
    }
}

/// Converts `S3PackageUri` to `ManifestUri`
/// `ManifestUri` should always have `hash`.
/// But `S3PackageUri` can be tagged (e.g. "latest" or timestamp).
/// So, we need to download the tag and find out what the `hash` is
pub async fn resolve_manifest_uri(
    remote: &impl Remote,
    host: &Option<Host>,
    uri: &S3PackageUri,
) -> Res<ManifestUri> {
    let bucket = uri.bucket.clone();
    let namespace = uri.namespace.clone();
    let hash = resolve_top_hash(remote, host, uri).await?;
    let origin = host.to_owned();
    Ok(ManifestUri {
        bucket,
        namespace,
        hash,
        origin,
    })
}

/// Upload file associated with manifest's `ManifestRow`.
/// After uploading we get new hash,
/// though it should be the same as already calclulated during commit.
/// Response with the new `ManifestRow` with `physical_key` pointing to the place it was uploaded to.
pub async fn upload_row(
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: S3PackageHandle,
    row: ManifestRow,
) -> Res<ManifestRow> {
    let local_url = Url::parse(&row.physical_key)?;
    if local_url.scheme() != "file" {
        return Err(Error::FileUri(local_url));
    }
    let file_path = local_url
        .to_file_path()
        .map_err(|_| Error::FileUri(local_url))?;

    let object_uri = ObjectUri::new(package_handle, row.logical_key.clone());
    log::info!("Uploading to S3: {object_uri}");

    let (remote_url, hash) = remote
        .upload_file(host_config, &file_path, &object_uri.into(), row.size)
        .await?;

    // Update the manifest with the sha2-256-chunked checksum
    // "Relax" the manifest by using those new remote keys
    let physical_key = remote_url.to_string();
    Ok(ManifestRow {
        hash,
        physical_key,
        ..row
    })
}

pub type StreamRowsChunk = Vec<Res<ManifestRow>>;

pub type StreamItem = Res<StreamRowsChunk>;

pub trait RowsStream: Stream<Item = StreamItem> {}

impl<T: Stream<Item = StreamItem>> RowsStream for T {}

/// Builds the manifest from `Stream<Result<Row>>`
/// It writes the manifest to temporary file using JSONL format.
/// Then it calclutates top_hash and move the temporary file to the destination path.
pub async fn build_manifest_from_rows_stream(
    storage: &impl Storage,
    dest_dir: PathBuf,
    header: ManifestHeader,
    mut stream: impl RowsStream + Unpin,
) -> Res<(PathBuf, String)> {
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path().join("manifest.jsonl");
    log::info!("Temp path for creating manifest {temp_path:?}");

    // Build manifest in memory
    let mut rows = Vec::new();
    let mut top_hasher = TopHasher::new();

    top_hasher.append_header(&header)?;

    while let Some(Ok(chunk)) = stream.next().await {
        for row_result in &chunk {
            match row_result {
                Ok(row) => {
                    top_hasher.append(row)?;
                    rows.push(row.clone());
                }
                Err(err) => return Err(Error::Table(err.to_string())),
            }
        }
    }

    // Create JSONL manifest
    let manifest = Manifest { header, rows };

    let jsonl_content = manifest.to_jsonlines();
    storage
        .write_byte_stream(&temp_path, jsonl_content.into_bytes().into())
        .await?;

    let top_hash = top_hasher.finalize();
    let dest_path = dest_dir.join(&top_hash);
    storage.create_dir_all(&dest_dir).await?;
    storage.rename(temp_path, &dest_path).await?;

    Ok((dest_path, top_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use tokio_stream;

    use crate::checksum::Crc64Hash;
    use crate::checksum::Sha256ChunkedHash;
    use crate::checksum::Sha256Hash;
    use crate::fixtures;
    use crate::fixtures::objects;
    use crate::fixtures::top_hash;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::LocalStorage;
    use crate::uri::S3Uri;

    #[test(tokio::test)]
    async fn test_resolve_existing_hash() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@hjknlmn")?;
        let remote = MockRemote::default();
        let top_hash = resolve_top_hash(&remote, &None, &uri).await?;
        assert_eq!(top_hash, "hjknlmn".to_string(),);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_resolve_remote_hash() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar")?;
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let top_hash = resolve_top_hash(&remote, &None, &uri).await?;
        assert_eq!(top_hash, "abcdef".to_string(),);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            Manifest::default().header,
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::EMPTY_EMPTY_TOP_HASH));
        assert_eq!(top_hash, top_hash::EMPTY_EMPTY_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_EMPTY_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::EMPTY_EMPTY_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty_none() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                user_meta: None,
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::EMPTY_NONE_TOP_HASH));
        assert_eq!(top_hash, top_hash::EMPTY_NONE_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NONE_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::EMPTY_NONE_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty_null() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                user_meta: Some(serde_json::Value::Null),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::EMPTY_NULL_TOP_HASH));
        assert_eq!(top_hash, top_hash::EMPTY_NULL_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NULL_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::EMPTY_NULL_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_null_empty() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: None,
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::NULL_EMPTY_TOP_HASH));
        assert_eq!(top_hash, top_hash::NULL_EMPTY_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::NULL_EMPTY_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::NULL_EMPTY_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_null_none() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: None,
                user_meta: None,
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::NULL_NONE_TOP_HASH));
        assert_eq!(top_hash, top_hash::NULL_NONE_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::NULL_NONE_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::NULL_NONE_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_null_null() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: None,
                user_meta: Some(serde_json::Value::Null),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::NULL_NULL_TOP_HASH));
        assert_eq!(top_hash, top_hash::NULL_NULL_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::NULL_NULL_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::NULL_NULL_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_empty() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::json!({})),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_EMPTY_TOP_HASH));
        assert_eq!(top_hash, top_hash::INITIAL_EMPTY_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_EMPTY_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::INITIAL_EMPTY_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_none() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: None,
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_NONE_TOP_HASH));
        assert_eq!(top_hash, top_hash::INITIAL_NONE_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_NONE_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::INITIAL_NONE_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_null() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::Value::Null),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_NULL_TOP_HASH));
        assert_eq!(top_hash, top_hash::INITIAL_NULL_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_NULL_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::INITIAL_NULL_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_meta() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::json!({"key": "value"})),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::INITIAL_META_TOP_HASH));
        assert_eq!(top_hash, top_hash::INITIAL_META_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_META_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::INITIAL_META_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_complex_meta() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::json!({"author": "user", "timestamp": "2024-01-01"})),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::INITIAL_COMPLEX_META_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::INITIAL_COMPLEX_META_TOP_HASH);

        // Create manifest from text content and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_COMPLEX_META_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::INITIAL_COMPLEX_META_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_large_meta() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let large_meta = serde_json::json!({
            "author": "user",
            "timestamp": "2024-01-01T10:30:00Z",
            "description": "This is a comprehensive test with larger metadata",
            "tags": ["test", "manifest", "quilt"],
            "version": 1,
            "nested": {
                "key1": "value1",
                "key2": 42,
                "key3": true
            }
        });
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(large_meta.clone()),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::INITIAL_LARGE_META_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::INITIAL_LARGE_META_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_LARGE_META_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::INITIAL_LARGE_META_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty_empty_simple_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("".to_string()),
                user_meta: Some(serde_json::json!({})),
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: None,
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty_empty_complex_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("".to_string()),
                user_meta: Some(serde_json::json!({})),
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: Some(WorkflowId {
                        id: "test-workflow".to_string(),
                        metadata: Some(MetadataSchema {
                            id: "test-schema".to_string(),
                            url: "s3://bucket/workflows/test.json".parse()?,
                        }),
                    }),
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_empty_simple_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::json!({})),
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: None,
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path =
            top_hash::load_fixture(top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_empty_complex_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::json!({})),
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: Some(WorkflowId {
                        id: "test-workflow".to_string(),
                        metadata: Some(MetadataSchema {
                            id: "test-schema".to_string(),
                            url: "s3://bucket/workflows/test.json".parse()?,
                        }),
                    }),
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path =
            top_hash::load_fixture(top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_checksummed_manifest_build_from_stream() -> Res {
        let storage = LocalStorage::default();
        let manifest = Manifest::from_path(&storage, &fixtures::manifest::path()?).await?;

        let mock_storage = MockStorage::default();
        let dest_dir = mock_storage.temp_dir.path();

        let rows_stream = tokio_stream::iter(vec![Ok(manifest.rows.into_iter().map(Ok).collect())]);

        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &mock_storage,
            dest_dir.to_path_buf(),
            manifest.header,
            rows_stream,
        )
        .await?;

        assert_eq!(calculated_hash, fixtures::manifest::TOP_HASH);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty_none_simple_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("".to_string()),
                user_meta: None,
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: None,
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_empty_null_simple_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("".to_string()),
                user_meta: Some(serde_json::Value::Null),
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: None,
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_meta_simple_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: Some(serde_json::json!({"key": "value"})),
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: None,
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path = top_hash::load_fixture(top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_empty_manifest_header_initial_none_complex_workflow() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let (dest_path, top_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: None,
                workflow: Some(Workflow {
                    config: "s3://workflow/config".parse()?,
                    id: Some(WorkflowId {
                        id: "test-workflow".to_string(),
                        metadata: Some(MetadataSchema {
                            id: "test-schema".to_string(),
                            url: "s3://bucket/workflows/test.json".parse()?,
                        }),
                    }),
                }),
                ..ManifestHeader::default()
            },
            tokio_stream::empty(),
        )
        .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH);

        // Create manifest from fixture file and verify top_hash matches
        let fixture_path =
            top_hash::load_fixture(top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH
        );
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_single_row_manifest() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let header = ManifestHeader::default();

        let manifest_row = ManifestRow {
            logical_key: PathBuf::from("data.txt"),
            physical_key: "s3://bucket/data.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"type": "text"})),
        };

        let rows_stream = tokio_stream::iter(vec![Ok(vec![Ok(manifest_row)])]);
        let (dest_path, top_hash) =
            build_manifest_from_rows_stream(&storage, dest_dir.to_path_buf(), header, rows_stream)
                .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::SINGLE_ROW_TOP_HASH));
        assert_eq!(top_hash, top_hash::SINGLE_ROW_TOP_HASH);

        // Verify using Manifest::from_path with the fixture file
        let fixture_path = top_hash::load_fixture(top_hash::SINGLE_ROW_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::SINGLE_ROW_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_mixed_hash_types_manifest() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let header = ManifestHeader::default();

        let row1 = ManifestRow {
            logical_key: PathBuf::from("file1.txt"),
            physical_key: "s3://bucket/file1.txt".to_string(),
            hash: Sha256Hash::try_from(
                "7465737464617461000000000000000000000000000000000000000000000000",
            )?
            .into(),
            size: 8,
            meta: None,
        };

        let row2 = ManifestRow {
            logical_key: PathBuf::from("file2.txt"),
            physical_key: "s3://bucket/file2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: None,
        };

        let row3 = ManifestRow {
            logical_key: PathBuf::from("file3.txt"),
            physical_key: "s3://bucket/file3.txt".to_string(),
            hash: Crc64Hash::try_from("dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA")?.into(),
            size: 32,
            meta: None,
        };

        let rows_stream = tokio_stream::iter(vec![Ok(vec![Ok(row1), Ok(row2), Ok(row3)])]);
        let (dest_path, top_hash) =
            build_manifest_from_rows_stream(&storage, dest_dir.to_path_buf(), header, rows_stream)
                .await?;
        assert_eq!(
            dest_path,
            dest_dir.join(top_hash::MIXED_HASH_TYPES_TOP_HASH)
        );
        assert_eq!(top_hash, top_hash::MIXED_HASH_TYPES_TOP_HASH);

        // Verify using Manifest::from_path with the fixture file
        let fixture_path = top_hash::load_fixture(top_hash::MIXED_HASH_TYPES_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash_from_reader) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(
            calculated_hash_from_reader,
            top_hash::MIXED_HASH_TYPES_TOP_HASH
        );
        assert_eq!(calculated_hash_from_reader, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_multiple_rows_manifest() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let header = ManifestHeader::default();

        let row1 = ManifestRow {
            logical_key: PathBuf::from("config.json"),
            physical_key: "s3://bucket/config.json".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::json!({"format": "json"})),
        };

        let row2 = ManifestRow {
            logical_key: PathBuf::from("data/file.csv"),
            physical_key: "s3://bucket/data/file.csv".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::EQUAL_TO_8MB_HASH_B64)?.into(),
            size: 8388608,
            meta: Some(serde_json::Value::Null),
        };

        let row3 = ManifestRow {
            logical_key: PathBuf::from("images/photo.jpg"),
            physical_key: "s3://bucket/images/photo.jpg".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::MORE_THAN_8MB_HASH_B64)?.into(),
            size: 18874368,
            meta: Some(serde_json::json!({"width": 1920, "height": 1080})),
        };

        let rows_stream = tokio_stream::iter(vec![Ok(vec![Ok(row1), Ok(row2), Ok(row3)])]);
        let (dest_path, top_hash) =
            build_manifest_from_rows_stream(&storage, dest_dir.to_path_buf(), header, rows_stream)
                .await?;
        assert_eq!(dest_path, dest_dir.join(top_hash::MULTIPLE_ROWS_TOP_HASH));
        assert_eq!(top_hash, top_hash::MULTIPLE_ROWS_TOP_HASH);

        // Verify using Manifest::from_path with the fixture file
        let fixture_path = top_hash::load_fixture(top_hash::MULTIPLE_ROWS_TOP_HASH)?;
        let local_storage = LocalStorage::default();
        let manifest = Manifest::from_path(&local_storage, &fixture_path).await?;
        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest.header.clone(),
            manifest.records_stream().await,
        )
        .await?;

        assert_eq!(calculated_hash, top_hash::MULTIPLE_ROWS_TOP_HASH);
        assert_eq!(calculated_hash, top_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_hash_normalization_equivalence_manifest() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path();
        let local_storage = LocalStorage::default();

        // Load all three variant fixture files
        let fixture_path1 = top_hash::load_equivalent_fixture(
            top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH,
            "canonical",
        )?;
        let fixture_path2 = top_hash::load_equivalent_fixture(
            top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH,
            "meta-null-key-order",
        )?;
        let fixture_path3 = top_hash::load_equivalent_fixture(
            top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH,
            "field-order-missing-meta",
        )?;

        // Load manifests from fixture files
        let manifest1 = Manifest::from_path(&local_storage, &fixture_path1).await?;
        let manifest2 = Manifest::from_path(&local_storage, &fixture_path2).await?;
        let manifest3 = Manifest::from_path(&local_storage, &fixture_path3).await?;

        // Calculate hashes for all three variants
        let (_, calculated_hash1) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest1.header.clone(),
            manifest1.records_stream().await,
        )
        .await?;

        let (_, calculated_hash2) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest2.header.clone(),
            manifest2.records_stream().await,
        )
        .await?;

        let (_, calculated_hash3) = build_manifest_from_rows_stream(
            &storage,
            dest_dir.to_path_buf(),
            manifest3.header.clone(),
            manifest3.records_stream().await,
        )
        .await?;

        // All three variants should produce the same hash despite different representations
        assert_eq!(
            calculated_hash1, calculated_hash2,
            "Canonical and meta-null-key-order variants should normalize to same hash"
        );
        assert_eq!(
            calculated_hash1, calculated_hash3,
            "Canonical and field-order-missing-meta variants should normalize to same hash"
        );
        assert_eq!(
            calculated_hash2, calculated_hash3,
            "All meta empty representations should normalize to same hash"
        );

        // Test that the normalized hash matches our expected constant
        assert_eq!(calculated_hash1, top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH);

        Ok(())
    }
}
