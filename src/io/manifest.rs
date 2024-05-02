use std::path::PathBuf;

use tracing::log;

use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::Table;
use crate::paths::get_manifest_key_legacy;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
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
        bucket: bucket,
        namespace: namespace,
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
    let tag_uri = TagUri::latest(manifest_uri);
    remote
        .put_object(&tag_uri.into(), manifest_uri.hash.as_bytes().to_vec())
        .await
}
