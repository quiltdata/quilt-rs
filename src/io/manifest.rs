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
use crate::manifest::Manifest;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::manifest::TopHasher;
use crate::paths::get_manifest_key_legacy;
use crate::uri::ManifestUri;
use crate::uri::ObjectUri;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageHandle;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::uri::TagUri;
use crate::Error;


async fn bytestream_to_string(bytestream: ByteStream) -> Result<String, Error> {
    let mut reader = bytestream.into_async_read();
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents).await?;
    String::from_utf8(contents).map_err(|err| Error::Utf8(err.utf8_error()))
}

async fn upload_legacy(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_path: &PathBuf,
    manifest_uri: &ManifestUri,
) -> Result<(), Error> {
    let s3uri = S3Uri {
        bucket: manifest_uri.bucket.clone(),
        key: get_manifest_key_legacy(&manifest_uri.hash),
        version: None,
    };
    remote
        .put_object(
            &s3uri,
            Manifest::from(&Table::read_from_path(storage, manifest_path).await?)
                .to_jsonlines()
                .as_bytes()
                .to_vec(),
        )
        .await
}

pub async fn upload_from(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_path: &PathBuf,
    manifest_uri: &ManifestUri,
) -> Result<(), Error> {
    // TODO: FAIL if the manifest with this hash already exists?
    let body = storage.read_byte_stream(manifest_path).await?;
    log::info!("Writing remote manifest to {:?}", manifest_uri);
    remote.put_object(&manifest_uri.into(), body).await
}

pub async fn upload_manifest(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    cache_path: &PathBuf,
) -> Result<(), Error> {
    // Push the (cached) relaxed manifest to the remote, don't tag it yet

    upload_from(storage, remote, cache_path, manifest_uri).await?;

    // Upload a quilt3 manifest for backward compatibility.
    upload_legacy(storage, remote, cache_path, manifest_uri).await?;

    log::debug!("Uploaded remote manifest: {:?}", manifest_uri);
    Ok(())
}

pub async fn tag_timestamp(
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<(), Error> {
    // Tag the new commit.
    // If {self.commit.tag} does not already exist at
    // {self.remote}/.quilt/named_packages/{self.namespace},
    // create it with the value of {self.commit.hash}
    // TODO: Otherwise try again with the current timestamp as the tag
    // (e.g., try five times with exponential backoff, then Error)
    let tag_uri = TagUri::timestamp(manifest_uri, timestamp);
    remote
        .put_object(&tag_uri.into(), manifest_uri.hash.as_bytes().to_vec())
        .await
}

pub async fn tag_latest(remote: &impl Remote, manifest_uri: &ManifestUri) -> Result<(), Error> {
    let tag_uri = TagUri::latest(&manifest_uri.into());
    remote
        .put_object(&tag_uri.into(), manifest_uri.hash.as_bytes().to_vec())
        .await
}

pub async fn resolve_top_hash(remote: &impl Remote, uri: S3PackageUri) -> Result<String, Error> {
    match &uri.revision {
        RevisionPointer::Hash(top_hash) => Ok(top_hash.clone()),
        RevisionPointer::Tag(_) => {
            let tag_uri = TagUri::latest(&uri);
            let stream = remote.get_object_stream(&tag_uri.into()).await?;
            bytestream_to_string(stream).await
        }
    }
}

pub async fn resolve_manifest_uri(
    remote: &impl Remote,
    uri: &S3PackageUri,
) -> Result<ManifestUri, Error> {
    let top_hash = match &uri.revision {
        RevisionPointer::Hash(top_hash) => top_hash.clone(),
        RevisionPointer::Tag(_) => {
            let tag_uri = TagUri::latest(&uri.clone());
            let stream = remote.get_object_stream(&tag_uri.into()).await?;
            bytestream_to_string(stream).await?
        }
    };
    Ok(ManifestUri {
        bucket: uri.bucket.clone(),
        namespace: uri.namespace.clone(),
        hash: top_hash,
    })
}

pub async fn resolve_latest(remote: &impl Remote, uri: S3PackageUri) -> Result<String, Error> {
    let tag_uri = TagUri::latest(&uri);
    let stream = remote.get_object_stream(&tag_uri.into()).await?;
    bytestream_to_string(stream).await
}

pub async fn upload_row(
    remote: &impl Remote,
    package_handle: S3PackageHandle,
    row: Row,
) -> Result<Row, Error> {
    let local_url = Url::parse(&row.place)?;
    if local_url.scheme() != "file" {
        return Err(Error::FileUri(local_url));
    }
    let file_path = local_url
        .to_file_path()
        .map_err(|_| Error::FileUri(local_url))?;

    let s3_uri = ObjectUri {
        bucket: package_handle.bucket,
        namespace: package_handle.namespace,
        path: row.name.clone(),
        version: None,
    };
    log::debug!("Uploading to S3: {}", s3_uri);

    let (remote_url, hash) = remote
        .upload_file(&file_path, &s3_uri.into(), row.size)
        .await?;

    // Update the manifest with the sha2-256-chunked checksum
    // "Relax" the manifest by using those new remote keys
    let place = remote_url.to_string();
    Ok(Row { hash, place, ..row })
}

pub enum ManifestTarget {
    Table(Table),
    File(File),
}

pub struct WritableManifest {
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
    pub async fn try_new(storage: &impl Storage, target: ManifestTarget) -> Result<Self, Error> {
        let file = match target {
            ManifestTarget::Table(_) => storage.open_file(PathBuf::new()).await?, // FIXME
            ManifestTarget::File(file) => file,
        };
        file.try_into()
    }

    pub async fn insert_record(&mut self, row: Row) -> Result<(), Error> {
        self.writer.insert_row(row).await
    }

    pub async fn flush(self) -> Result<(), Error> {
        self.writer.flush().await
    }
}

pub async fn build_manifest_from_rows_stream(
    storage: &impl Storage,
    manifest_path: impl Fn(&str) -> PathBuf,
    header: Row,
    mut stream: impl Stream<Item = Result<Row, Error>> + std::marker::Unpin,
) -> Result<(PathBuf, String), Error> {
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path().join("manifest.pq");
    let file = storage.create_file(&temp_path).await?;
    let mut manifest = WritableManifest::try_new(storage, file.into()).await?;

    let mut top_hasher = TopHasher::new();
    top_hasher.append(&header)?;
    manifest.insert_record(header).await?;

    while let Some(Ok(row)) = stream.next().await {
        top_hasher.append(&row)?;
        manifest.insert_record(row).await?;
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
    async fn test_resolve_existing_hash() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@hjknlmn")?;
        let remote = mocks::remote::MockRemote::default();
        let top_hash = resolve_top_hash(&remote, uri).await?;
        assert_eq!(top_hash, "hjknlmn".to_string(),);
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_remote_hash() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar")?;
        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let top_hash = resolve_top_hash(&remote, uri).await?;
        assert_eq!(top_hash, "abcdef".to_string(),);
        Ok(())
    }
}
