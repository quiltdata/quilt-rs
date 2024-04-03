use std::path::PathBuf;

use arrow::error::ArrowError;
use aws_sdk_s3::error::{DisplayErrorContext, SdkError};
use storage::fs::LocalStorage;
use storage::Storage;
use tokio::{fs, io::AsyncReadExt};

use crate::{paths, Table, UPath};
use crate::{
    quilt::{
        manifest::Manifest,
        manifest_handle::{CachedManifest, ReadableManifest, RemoteManifest},
        storage, Error,
    },
    s3_utils,
};

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
        Err(err) => Err(Error::S3(DisplayErrorContext(err).to_string())),
    }
}

async fn fetch_parquet(
    client: &aws_sdk_s3::Client,
    manifest: &RemoteManifest,
) -> Result<Vec<u8>, Error> {
    let key = paths::get_manifest_key(&manifest.hash);
    let mut contents = s3_utils::get_object(client, &manifest.bucket, &key).await?;
    let mut output = Vec::new();
    contents.read_to_end(&mut output).await?;
    Ok(output)
}

async fn fetch_jsonl(
    client: &aws_sdk_s3::Client,
    manifest: &RemoteManifest,
) -> Result<Table, Error> {
    let key = paths::get_manifest_key_legacy(&manifest.hash);
    let contents = s3_utils::get_object(client, &manifest.bucket, &key).await?;
    let quilt3_manifest = Manifest::from_reader(contents).await?;
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
    remote_manifest: &RemoteManifest,
) -> Result<CachedManifest, Error> {
    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    let cache_path = paths.manifest_cache(&remote_manifest.bucket, &remote_manifest.hash);
    let storage = LocalStorage::new(paths.working_dir(&remote_manifest.namespace));

    if !storage.exists(&cache_path).await {
        // Does not exist yet
        let client = crate::s3_utils::get_client_for_bucket(&remote_manifest.bucket).await?;
        if is_parquet(&client, remote_manifest).await? {
            let manifest = fetch_parquet(&client, remote_manifest).await?;
            storage::fs::write(&cache_path, &manifest).await?;
        } else {
            let manifest = fetch_jsonl(&client, remote_manifest).await?;
            manifest.write_to_upath(&UPath::Local(cache_path)).await?;
        };
    }

    Ok(CachedManifest::from_remote_manifest(remote_manifest, paths))
}

pub async fn browse_remote_manifest(
    paths: &paths::DomainPaths,
    remote: &RemoteManifest,
) -> Result<Table, Error> {
    cache_remote_manifest(paths, remote).await?.read().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use temp_testdir::TempDir;

    #[tokio::test]
    async fn test_if_cached() -> Result<(), Error> {
        let root_dir = TempDir::default();
        let paths = paths::DomainPaths::new(root_dir.to_path_buf());
        let manifest = RemoteManifest {
            bucket: "a".to_string(),
            namespace: "b".to_string(),
            hash: "c".to_string(),
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        storage::fs::write(cache_path, &(Vec::new())).await?;
        let cached_manifest = cache_remote_manifest(&paths, &manifest).await?;
        assert_eq!(
            cached_manifest,
            CachedManifest {
                paths,
                bucket: "a".to_string(),
                hash: "c".to_string(),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_if_cached_random_file() -> Result<(), Error> {
        let root_dir = TempDir::default();
        let paths = paths::DomainPaths::new(root_dir.to_path_buf());
        let manifest = RemoteManifest {
            bucket: "a".to_string(),
            namespace: "b".to_string(),
            hash: "c".to_string(),
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        storage::fs::write(cache_path, &(Vec::new())).await?;
        let cached_manifest = cache_remote_manifest(&paths, &manifest).await?;
        assert_eq!(
            cached_manifest.read().await.unwrap_err().to_string(),
            "Arrow error: Parquet argument error: External: Invalid argument (os error 22)"
        );
        Ok(())
    }
}
