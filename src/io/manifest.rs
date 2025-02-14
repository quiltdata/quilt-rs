//! Contains utility functions to work with manifests.

use std::marker::Unpin;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tracing::log;
use url::Url;

use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::io::ParquetWriter;
use crate::manifest::Header;
use crate::manifest::Manifest;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::manifest::TopHasher;
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::ManifestUriLegacy;
use crate::uri::ObjectUri;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageHandle;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::uri::TagUri;
use crate::Error;
use crate::Res;

async fn bytestream_to_string(bytestream: ByteStream) -> Res<String> {
    let mut reader = bytestream.into_async_read();
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents).await?;
    String::from_utf8(contents).map_err(|err| Error::Utf8(err.utf8_error()))
}

/// Read Parquet file and upload manifest converted to JSONL.
/// We don't care about checksum of the resulted file.
async fn upload_legacy(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_path: &PathBuf,
    manifest_uri: &ManifestUri,
) -> Res {
    let s3_uri: S3Uri = ManifestUriLegacy::from(manifest_uri).into();
    let jsonl = Manifest::from_table(&Table::read_from_path(storage, manifest_path).await?)
        .await?
        .to_jsonlines()
        .as_bytes()
        .to_vec();
    remote
        .put_object(&manifest_uri.catalog, &s3_uri, jsonl)
        .await
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
    log::info!("Writing remote manifest to {:?}", manifest_uri);
    remote
        .put_object(&manifest_uri.catalog, &manifest_uri.into(), body)
        .await
}

/// Upload manifest with both formats JSONL and Parquet.
/// We don't care about checksum of the resulted files.
pub async fn upload_manifest(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    path: &PathBuf,
) -> Res {
    // Push the (cached) relaxed manifest to the remote, don't tag it yet
    upload_from(storage, remote, path, manifest_uri).await?;
    log::info!("Parque file uploaded");

    // Upload a quilt3 manifest for backward compatibility.
    upload_legacy(storage, remote, path, manifest_uri).await?;
    log::info!("JSONL file uploaded");

    log::info!("Uploaded remote manifest: {:?}", manifest_uri);
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
            &manifest_uri.catalog,
            &tag_uri.into(),
            manifest_uri.hash.as_bytes().to_vec(),
        )
        .await
}

/// Downloads the latest tagged package
/// and returns its content: hash of the latest package revision.
/// Then creates `ManifestUri`.
pub async fn resolve_latest(
    remote: &impl Remote,
    host: &Host,
    uri: S3PackageHandle,
) -> Res<ManifestUri> {
    let tag_uri = TagUri::latest(uri.clone());
    let stream = remote.get_object_stream(host, &tag_uri.into()).await?;
    let hash = bytestream_to_string(stream.body).await?;
    Ok(ManifestUri {
        hash,
        bucket: uri.bucket,
        namespace: uri.namespace,
        catalog: host.clone(),
    })
}

/// `ManifestUri` should always have `hash`.
/// But `S3PackageUri` can be just tagged as "latest".
/// So, we need to dowload "latest" tag and find out what the `hash` is
async fn resolve_top_hash(remote: &impl Remote, host: &Host, uri: &S3PackageUri) -> Res<String> {
    match &uri.revision {
        RevisionPointer::Hash(top_hash) => Ok(top_hash.clone()),
        RevisionPointer::Tag(_) => Ok(resolve_latest(remote, host, uri.into()).await?.hash),
    }
}

/// Converts `S3PackageUri` to `ManifestUri`
/// `ManifestUri` should always have `hash`.
/// But `S3PackageUri` can be just tagged as "latest".
/// So, we need to dowload "latest" tag and find out what the `hash` is
pub async fn resolve_manifest_uri(
    remote: &impl Remote,
    host: &Host,
    uri: &S3PackageUri,
) -> Res<ManifestUri> {
    let bucket = uri.bucket.clone();
    let namespace = uri.namespace.clone();
    let hash = resolve_top_hash(remote, host, uri).await?;
    Ok(ManifestUri {
        bucket,
        namespace,
        hash,
        catalog: host.clone(),
    })
}

/// Upload file associated with manifest's `Row`.
/// After uploading we get new hash,
/// though it should be the same as already calclulated during commit.
/// Response with the new `Row` with `place` pointing to the place it was uploaded to.
pub async fn upload_row(
    remote: &impl Remote,
    host: &Host,
    package_handle: S3PackageHandle,
    row: Row,
) -> Res<Row> {
    let local_url = Url::parse(&row.place)?;
    if local_url.scheme() != "file" {
        return Err(Error::FileUri(local_url));
    }
    let file_path = local_url
        .to_file_path()
        .map_err(|_| Error::FileUri(local_url))?;

    let object_uri = ObjectUri::new(package_handle, row.name.clone());
    log::info!("Uploading to S3: {}", object_uri);

    let (remote_url, hash) = remote
        .upload_file(host, &file_path, &object_uri.into(), row.size)
        .await?;

    // Update the manifest with the sha2-256-chunked checksum
    // "Relax" the manifest by using those new remote keys
    let place = remote_url.to_string();
    Ok(Row { hash, place, ..row })
}

enum ManifestTarget {
    Table(Table),
    File(File),
}

// This is
struct WritableManifest {
    writer: ParquetWriter,
}

impl From<File> for ManifestTarget {
    fn from(file: File) -> Self {
        ManifestTarget::File(file)
    }
}

impl From<Table> for ManifestTarget {
    fn from(manifest: Table) -> Self {
        ManifestTarget::Table(manifest)
    }
}

impl TryFrom<File> for WritableManifest {
    type Error = Error;

    fn try_from(file: File) -> Result<Self, Self::Error> {
        Ok(WritableManifest {
            writer: file.try_into()?,
        })
    }
}

impl WritableManifest {
    pub async fn try_new(storage: &impl Storage, target: ManifestTarget) -> Res<Self> {
        let file = match target {
            ManifestTarget::Table(_table) => storage.open_file(PathBuf::new()).await?, // FIXME
            ManifestTarget::File(file) => file,
        };
        file.try_into()
    }

    pub async fn insert_header(&mut self, header: Header) -> Res {
        let header_chunk: StreamRowsChunk = vec![Ok(header.into())];
        self.writer.insert(header_chunk).await
    }

    pub async fn insert(&mut self, chunk: StreamRowsChunk) -> Res {
        self.writer.insert(chunk).await
    }

    /// Close and finalize the writer.
    pub async fn flush(self) -> Res {
        self.writer.flush().await
    }
}

pub type StreamRowsChunk = Vec<Res<Row>>;

pub type StreamItem = Res<StreamRowsChunk>;

pub trait RowsStream: Stream<Item = StreamItem> {}

impl<T: Stream<Item = StreamItem>> RowsStream for T {}

/// Builds the manifest from `Stream<Result<Row>>`
/// It writes the manifest to temporary file using Parquet.
/// Then it calclutates top_hash and move the temporary file to the destination path.
pub async fn build_manifest_from_rows_stream(
    storage: &impl Storage,
    manifest_path: impl Fn(&str) -> PathBuf,
    header: Header,
    mut stream: impl RowsStream + Unpin,
) -> Res<(PathBuf, String)> {
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path().join("manifest.pq");
    log::info!("Temp path for creating manifest {:?}", temp_path);
    let file = storage.create_file(&temp_path).await?;
    let mut manifest = WritableManifest::try_new(storage, file.into()).await?;

    let mut top_hasher = TopHasher::new();
    top_hasher.append_header(&header)?;
    manifest.insert_header(header).await?;

    while let Some(Ok(rows)) = stream.next().await {
        for row in &rows {
            match row {
                Ok(row) => top_hasher.append(row)?,
                Err(err) => return Err(Error::Table(err.to_string())),
            }
        }
        manifest.insert(rows).await?;
    }
    manifest.flush().await?;

    let top_hash = top_hasher.finalize();
    let dest_path = manifest_path(&top_hash);
    storage.create_dir_all(&dest_path.parent().unwrap()).await?;
    storage.rename(temp_path, &dest_path).await?;

    Ok((dest_path, top_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::mocks;

    #[tokio::test]
    async fn test_resolve_existing_hash() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@hjknlmn")?;
        let remote = mocks::remote::MockRemote::default();
        let top_hash = resolve_top_hash(&remote, &Host::default(), &uri).await?;
        assert_eq!(top_hash, "hjknlmn".to_string(),);
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_remote_hash() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar")?;
        let remote = mocks::remote::MockRemote::default();
        let host = Host::default();
        remote
            .put_object(
                &host,
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let top_hash = resolve_top_hash(&remote, &host, &uri).await?;
        assert_eq!(top_hash, "abcdef".to_string(),);
        Ok(())
    }
}
