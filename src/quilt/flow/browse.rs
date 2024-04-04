use std::path::PathBuf;

use arrow::error::ArrowError;
use tokio::io::AsyncReadExt;

use crate::paths;
use crate::quilt::manifest::Manifest;
use crate::quilt::manifest_handle::CachedManifest;
use crate::quilt::manifest_handle::ReadableManifest;
use crate::quilt::manifest_handle::RemoteManifest;
use crate::quilt::remote::Remote;
use crate::quilt::storage::Storage;
use crate::quilt::Error;
use crate::Table;
use crate::UPath;

async fn is_parquet(remote: &impl Remote, manifest: &RemoteManifest) -> Result<bool, Error> {
    remote
        .exists(&manifest.bucket, &paths::get_manifest_key(&manifest.hash))
        .await
}

async fn fetch_parquet(remote: &impl Remote, manifest: &RemoteManifest) -> Result<Vec<u8>, Error> {
    let key = paths::get_manifest_key(&manifest.hash);
    let mut contents = remote.get_object(&manifest.bucket, &key).await?;
    let mut output = Vec::new();
    contents.read_to_end(&mut output).await?;
    Ok(output)
}

async fn fetch_jsonl(remote: &impl Remote, manifest: &RemoteManifest) -> Result<Table, Error> {
    let key = paths::get_manifest_key_legacy(&manifest.hash);
    let contents = remote.get_object(&manifest.bucket, &key).await?;
    let quilt3_manifest = Manifest::from_reader(contents).await?;
    Table::try_from(quilt3_manifest)
}

pub async fn cache_manifest(
    paths: &paths::DomainPaths,
    storage: &mut impl Storage,
    manifest: &Table,
    bucket: &str,
    hash: &str,
) -> Result<PathBuf, ArrowError> {
    let cache_path = paths.manifest_cache(bucket, hash);
    storage
        .create_dir_all(&cache_path.parent().unwrap())
        .await?;
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
    storage: &mut impl Storage,
    remote: &impl Remote,
    remote_manifest: &RemoteManifest,
) -> Result<CachedManifest, Error> {
    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    let cache_path = paths.manifest_cache(&remote_manifest.bucket, &remote_manifest.hash);

    if !storage.exists(&cache_path).await {
        // Does not exist yet
        if is_parquet(remote, remote_manifest).await? {
            let manifest = fetch_parquet(remote, remote_manifest).await?;
            storage.write(cache_path.clone(), &manifest).await?;
        } else {
            let manifest = fetch_jsonl(remote, remote_manifest).await?;
            manifest.write_to_upath(&UPath::Local(cache_path)).await?;
        };
    }

    Ok(CachedManifest::from_remote_manifest(remote_manifest, paths))
}

pub async fn browse_remote_manifest(
    paths: &paths::DomainPaths,
    storage: &mut impl Storage,
    remote: &impl Remote,
    remote_manifest: &RemoteManifest,
) -> Result<Table, Error> {
    cache_remote_manifest(paths, storage, remote, remote_manifest)
        .await?
        .read()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use temp_testdir::TempDir;

    use crate::quilt::storage::fs::LocalStorage;
    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::s3_utils;

    #[tokio::test]
    async fn test_if_cached() -> Result<(), Error> {
        let root_dir = TempDir::default();
        let mut storage = MockStorage::default();
        let paths = paths::DomainPaths::new(root_dir.to_path_buf());
        let manifest = RemoteManifest {
            bucket: "a".to_string(),
            namespace: "b".to_string(),
            hash: "c".to_string(),
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        storage.write(cache_path, &(Vec::new())).await?;
        let remote = s3_utils::RemoteS3::new();
        let cached_manifest =
            cache_remote_manifest(&paths, &mut storage, &remote, &manifest).await?;
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
        // TODO: Can't use MockStorage because of parquet file reader
        let mut storage = LocalStorage::default();
        let paths = paths::DomainPaths::new(root_dir.to_path_buf());
        let manifest = RemoteManifest {
            bucket: "a".to_string(),
            namespace: "b".to_string(),
            hash: "c".to_string(),
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        storage.write(cache_path, &(Vec::new())).await?;
        let remote = s3_utils::RemoteS3::new();
        let cached_manifest =
            cache_remote_manifest(&paths, &mut storage, &remote, &manifest).await?;
        assert_eq!(
            cached_manifest.read().await.unwrap_err().to_string(),
            "Arrow error: Parquet argument error: External: Invalid argument (os error 22)"
        );
        Ok(())
    }
}
