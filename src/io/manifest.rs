use std::path::PathBuf;

use tracing::log;
use url::Url;

use crate::io::remote::utils::bytestream_to_string;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths::get_manifest_key_legacy;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::ObjectUri;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::uri::TagUri;
use crate::Error;

async fn cache_manifest(
    paths: &DomainPaths,
    storage: &impl Storage,
    manifest: &Table,
    bucket: &str,
) -> Result<(PathBuf, String), Error> {
    scaffold_paths(storage, paths.required_local_domain_paths()).await?;
    let top_hash = manifest.top_hash();
    let cache_path = paths.manifest_cache(bucket, &top_hash);
    storage
        .create_dir_all(&cache_path.parent().unwrap())
        .await?;
    manifest.write_to_path(storage, &cache_path).await?;
    Ok((cache_path, top_hash))
}

async fn upload_legacy(
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    table: &Table,
) -> Result<(), Error> {
    let s3uri = S3Uri {
        bucket: manifest_uri.bucket.clone(),
        key: get_manifest_key_legacy(&manifest_uri.hash),
        version: None,
    };
    remote
        .put_object(
            &s3uri,
            Manifest::from(table).to_jsonlines().as_bytes().to_vec(),
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
    paths: &DomainPaths,
    bucket: String,
    namespace: Namespace,
    manifest: Table,
) -> Result<ManifestUri, Error> {
    let (cache_path, top_hash) = cache_manifest(paths, storage, &manifest, &bucket).await?;

    let manifest_uri = ManifestUri {
        bucket,
        namespace,
        hash: top_hash,
    };

    // Push the (cached) relaxed manifest to the remote, don't tag it yet

    upload_from(storage, remote, &cache_path, &manifest_uri).await?;

    // Upload a quilt3 manifest for backward compatibility.
    upload_legacy(remote, &manifest_uri, &manifest).await?;

    log::debug!("Uploaded remote manifest: {:?}", manifest_uri);
    Ok(manifest_uri)
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
    manifest_uri: ManifestUri,
    row: &mut Row,
) -> Result<(), Error> {
    let local_url = Url::parse(&row.place)?;
    if local_url.scheme() != "file" {
        return Err(Error::FileUri(local_url));
    }
    let file_path = local_url
        .to_file_path()
        .map_err(|_| Error::FileUri(local_url))?;

    let s3_uri = ObjectUri {
        bucket: manifest_uri.bucket.clone(),
        namespace: manifest_uri.namespace.clone(),
        path: row.name.clone(),
        version: None,
    };
    log::debug!("Uploading to S3: {}", s3_uri);

    let (remote_url, hash) = remote
        .upload_file(&file_path, &s3_uri.into(), row.size)
        .await?;

    // Update the manifest with the sha2-256-chunked checksum
    row.hash = hash;
    // "Relax" the manifest by using those new remote keys
    row.place = remote_url.to_string();
    Ok(())
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
