use tokio_stream::StreamExt;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Header;
use crate::manifest::Manifest;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::ManifestUriLegacy;
use crate::uri::S3Uri;
use crate::Res;

async fn stream_jsonl_rows(jsonl: Manifest) -> impl RowsStream {
    tokio_stream::iter(jsonl.rows)
        .map(Row::try_from)
        .map(|rows| Ok(vec![rows]))
}

async fn is_parquet(remote: &impl Remote, manifest: &ManifestUri) -> Res<bool> {
    remote.exists(&S3Uri::from(manifest)).await
}

async fn fetch_parquet(remote: &impl Remote, manifest: &ManifestUri) -> Res<Vec<u8>> {
    let s3_uri = S3Uri::from(manifest);
    Ok(remote
        .get_object_stream(&s3_uri)
        .await?
        .stream
        .collect()
        .await?
        .to_vec())
}

async fn fetch_jsonl(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res<Manifest> {
    let s3_uri: S3Uri = ManifestUriLegacy::from(manifest_uri).into();
    let contents = remote
        .get_object_stream(&s3_uri)
        .await?
        .stream
        .into_async_read();
    Manifest::from_reader(contents).await
}

/// If remote manifest is already cached we return it.
/// If it's not cached, downloads the remote manifest and put it into cached directory.
/// If the manifest is in Parquet format we just put it as a file unchanged.
/// If the manifest is in JSONL format we read it and convert into Parquet, then write.
/// You must provide `ManifestUri` (package URI with `hash`).
/// To resolve `latest` you can use `manifest.io.resolve_latest`.
pub async fn cache_remote_manifest(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Res<Table> {
    scaffold_paths(storage, paths.required_local_domain_paths()).await?;
    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    // let manifest_uri = resolve_manifest_uri(remote, uri).await?;
    let cache_path = paths.manifest_cache(&manifest_uri.bucket, &manifest_uri.hash);

    if !storage.exists(&cache_path).await {
        // Does not exist yet
        if is_parquet(remote, manifest_uri).await? {
            let manifest = fetch_parquet(remote, manifest_uri).await?;
            storage.write_file(&cache_path, &manifest).await?;
        } else {
            let manifest = fetch_jsonl(remote, manifest_uri).await?;
            let header = Header::from(&manifest);
            let manifest_path = |_: &str| cache_path.clone();
            let stream = stream_jsonl_rows(manifest).await;
            build_manifest_from_rows_stream(storage, manifest_path, header, stream).await?;
        };
    }

    Table::read_from_path(storage, &cache_path).await
}

/// Alias for the `cache_remote_manifest`.
/// So, when we "browse" remote manifest, we always cache it.
pub async fn browse_remote_manifest(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Res<Table> {
    cache_remote_manifest(paths, storage, remote, manifest_uri).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::mocks;

    #[tokio::test]
    async fn test_if_cached() -> Res {
        let paths = DomainPaths::default();
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "c".to_string(),
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        let parquet = std::fs::read(mocks::manifest::parquet())?;
        let storage = mocks::storage::MockStorage::default();
        storage.write_file(cache_path, &parquet).await?;
        let remote = mocks::remote::MockRemote::default();
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await?;
        assert_eq!(
            cached_manifest.header.info.get("message").unwrap(),
            "test_spec_write 2023-11-29T14:01:39.543975"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_if_cached_random_file() -> Res {
        let paths = DomainPaths::default();
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "c".to_string(),
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        let storage = mocks::storage::MockStorage::default();
        storage.write_file(cache_path, &Vec::new()).await?;
        let remote = mocks::remote::MockRemote::default();
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await;
        assert_eq!(
            cached_manifest.unwrap_err().to_string(),
            "Parquet error: External: Invalid argument (os error 22)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_caching_parquet() -> Res {
        let storage = mocks::storage::MockStorage::default();
        let paths = DomainPaths::default();
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "c".to_string(),
        };
        let parquet = std::fs::read(mocks::manifest::parquet())?;
        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://a/.quilt/packages/1220c.parquet")?,
                parquet.clone(),
            )
            .await?;
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await?;
        assert_eq!(
            storage
                .read_file(&PathBuf::from(".quilt/packages/a/c"))
                .await?,
            parquet
        );
        assert_eq!(
            cached_manifest.header.info.get("message").unwrap(),
            "test_spec_write 2023-11-29T14:01:39.543975"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_caching_jsonl() -> Res {
        let storage = mocks::storage::MockStorage::default();
        let paths = DomainPaths::default();
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "c".to_string(),
        };
        let jsonl = std::fs::read(mocks::manifest::jsonl())?;
        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(&S3Uri::try_from("s3://a/.quilt/packages/c")?, jsonl.clone())
            .await?;
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await?;
        assert!(storage.exists(&PathBuf::from(".quilt/packages/a/c")).await);
        assert!(cached_manifest
            .get_record(&PathBuf::from("README.md"))
            .await?
            .is_some());
        Ok(())
    }
}
