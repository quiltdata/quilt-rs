use std::path::PathBuf;

use arrow::error::ArrowError;
use aws_sdk_s3::error::SdkError;
use tokio::{fs, io::AsyncReadExt};

use crate::quilt::{
    manifest::Manifest,
    manifest_handle::{CachedManifest, ReadableManifest, RemoteManifest},
    storage, Error,
};
use crate::{paths, Table, UPath};

async fn is_parquet(client: &aws_sdk_s3::Client, manifest: &RemoteManifest) -> Result<bool, Error> {
    match client
        .head_object()
        .bucket(&manifest.bucket)
        .key(paths::get_manifest_key(&manifest.hash))
        .send()
        .await
    {
        Ok(_) => Ok(true),
        Err(SdkError::ServiceError(err)) if err.err().is_not_found() => Ok(false),
        Err(err) => Err(Error::S3(err.to_string())),
    }
}

async fn fetch_parquet(
    client: &aws_sdk_s3::Client,
    manifest: &RemoteManifest,
) -> Result<Vec<u8>, Error> {
    let result = client
        .get_object()
        .bucket(&manifest.bucket)
        .key(paths::get_manifest_key(&manifest.hash))
        .send()
        .await
        .map_err(|err| Error::S3(aws_sdk_s3::error::DisplayErrorContext(err).to_string()))?;
    let mut contents = Vec::new();
    result
        .body
        .into_async_read()
        .read_to_end(&mut contents)
        .await?;
    Ok(contents)
}

async fn fetch_jsonl(
    client: &aws_sdk_s3::Client,
    manifest: &RemoteManifest,
) -> Result<Table, Error> {
    let result = client
        .get_object()
        .bucket(&manifest.bucket)
        .key(paths::get_manifest_key_legacy(&manifest.hash))
        .send()
        .await
        .map_err(|err| Error::S3(aws_sdk_s3::error::DisplayErrorContext(err).to_string()))?;
    let quilt3_manifest = Manifest::from_file(result.body.into_async_read()).await?;
    Table::try_from(quilt3_manifest)
}

pub async fn cache_manifest(
    paths: &paths::DomainPaths,
    manifest: &Table,
    bucket: &str,
    hash: &str,
) -> Result<PathBuf, ArrowError> {
    let cache_path = paths.manifest_cache(bucket, hash);
    fs::create_dir_all(&cache_path.parent().unwrap()).await?;
    manifest
        .write_to_upath(&UPath::Local(cache_path.clone()))
        .await
        .map(|_| cache_path)
}

// FIXME: CachedManifest::browse(&RemoteManifest)
//        or RemoteManifest::browse -> CachedManifest
//        or CachedManifest::try_from(RemoteManifest)
pub async fn cache_remote_manifest(
    paths: &paths::DomainPaths,
    manifest: &RemoteManifest,
) -> Result<impl ReadableManifest, Error> {
    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);

    // TODO: who is responsible for this?
    fs::create_dir_all(&cache_path.parent().unwrap()).await?;

    if !storage::fs::exists(&cache_path).await {
        // Does not exist yet
        let client = crate::s3_utils::get_client_for_bucket(&manifest.bucket).await?;
        if is_parquet(&client, manifest).await? {
            let output = fetch_parquet(&client, manifest).await?;
            storage::fs::write(&cache_path, &output).await?;
        } else {
            let table = fetch_jsonl(&client, manifest).await?;
            table.write_to_upath(&UPath::Local(cache_path)).await?;
        };
    }

    Ok(CachedManifest {
        paths: paths.clone(),
        bucket: manifest.bucket.clone(),
        hash: manifest.hash.clone(),
    })
}

pub async fn browse_remote_manifest(
    paths: &paths::DomainPaths,
    remote: &RemoteManifest,
) -> Result<Table, Error> {
    cache_remote_manifest(paths, remote).await?.read().await
}
