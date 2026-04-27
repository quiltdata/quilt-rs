use aws_sdk_s3::primitives::ByteStream;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::error::ManifestError;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use crate::Error;
use crate::Res;
use quilt_uri::ManifestUri;
use quilt_uri::S3Uri;

async fn fetch_jsonl(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res<Manifest> {
    let s3_uri: S3Uri = manifest_uri.clone().into();
    let contents = remote
        .get_object_stream(&manifest_uri.origin, &s3_uri)
        .await?;
    Manifest::from_reader(contents.body.into_async_read()).await
}

async fn fetch_and_cache(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    cache_path: &std::path::Path,
) -> Res<Manifest> {
    debug!(
        "⏳ Fetching JSONL manifest {} from remote…",
        manifest_uri.display()
    );
    let manifest = fetch_jsonl(remote, manifest_uri).await?;
    debug!("✔️ Fetched JSONL manifest");

    storage
        .write_byte_stream(cache_path, ByteStream::from(&manifest))
        .await?;
    debug!("✔️ JSONL manifest written to {}", cache_path.display());
    Ok(manifest)
}

/// If remote manifest is already cached we return it.
/// If it's not cached, downloads the remote JSONL manifest and stores it in cache.
/// You must provide `ManifestUri` (package URI with `hash`).
/// To resolve `latest` you can use `manifest.io.resolve_tag` with `Tag::Latest`.
pub async fn cache_remote_manifest(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Res<Manifest> {
    info!("⏳ Caching remote manifest: {}", manifest_uri.display());

    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    // let manifest_uri = resolve_manifest_uri(remote, uri).await?;
    let cache_dir = paths.cached_manifests_dir(&manifest_uri.bucket);
    let cache_path = cache_dir.join(&manifest_uri.hash);

    let manifest_path = cache_path.clone();

    if !storage.exists(&cache_path).await {
        let manifest = fetch_and_cache(storage, remote, manifest_uri, &cache_path).await?;
        info!("✔️ Successfully cached:\n{:?}", manifest.header);
        return Ok(manifest);
    }

    debug!("✔️ Manifest exists already in {}", cache_path.display());

    match Manifest::from_path(storage, &manifest_path).await {
        Ok(manifest) => {
            info!("✔️ Successfully cached:\n{:?}", manifest.header);
            Ok(manifest)
        }
        Err(Error::Manifest(ManifestError::Load { source, .. })) => {
            // Cached file is unreadable (e.g. legacy Parquet format), re-fetch
            warn!(
                "Cached manifest at {} is invalid, re-fetching: {}",
                manifest_path.display(),
                source
            );
            storage.remove_file(&manifest_path).await?;
            fetch_and_cache(storage, remote, manifest_uri, &cache_path).await
        }
        Err(e) => Err(e),
    }
}

/// Alias for the `cache_remote_manifest`.
/// So, when we "browse" remote manifest, we always cache it.
pub async fn browse_remote_manifest(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Res<Manifest> {
    cache_remote_manifest(paths, storage, remote, manifest_uri).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::str::FromStr;

    use test_log::test;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::LocalStorage;
    use crate::io::storage::StorageExt;

    /// Verifies that when a manifest is already cached,
    /// the `browse_remote_manifest` function retrieves it from the cache
    /// without making calls to the remote storage.
    #[test(tokio::test)]
    async fn test_if_cached() -> Res {
        let paths = DomainPaths::default();

        // Determine the expected cache path for this manifest.
        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "deadbeef".to_string(), // We use it for path location, but it's not used for verification
            origin: None,
        };
        let cache_path = paths.cached_manifest(&manifest_uri);

        // Prepare the reference manifest file.
        // It is copied into the cache path to simulate a cached manifest.
        let jsonl = ByteStream::from_path(fixtures::manifest::path()?).await?;
        let storage = MockStorage::default();
        storage.write_byte_stream(&cache_path, jsonl).await?;

        // Although there is no direct assertion for `remote.expect_get_object().never()`,
        // we know the remote is not called because a missing key would throw an error.
        let remote = MockRemote::default();

        // Since the manifest is cached, it should be retrieved from the cache
        // without any remote interaction.
        let cached_manifest =
            cache_remote_manifest(&paths, &storage, &remote, &manifest_uri).await?;

        // Verify that the cached manifest matches the reference manifest
        // JSONL fixture has no message but has user_meta set to null
        assert_eq!(cached_manifest.header.user_meta, None);

        Ok(())
    }

    /// Verifies that when a manifest is already cached but contains invalid data,
    /// the `browse_remote_manifest` function responds with an appropriate error.
    #[test(tokio::test)]
    async fn test_if_cached_random_file() -> Res {
        let paths = DomainPaths::default();

        // Simulate a corrupted cached manifest.
        // Write invalid data (an empty vector) to the cache path.
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "invalid_hash".to_string(),
            origin: None,
        };
        let cache_path = paths.cached_manifest(&manifest);
        let storage = MockStorage::default();
        storage
            .write_byte_stream(cache_path, ByteStream::default())
            .await?;

        let remote = MockRemote::default();

        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await;

        assert!(cached_manifest.is_err());

        Ok(())
    }

    /// Verifies that when a cached manifest is invalid but the remote
    /// has a valid JSONL manifest, the stale cache is replaced and
    /// the manifest is returned successfully.
    #[test(tokio::test)]
    async fn test_if_cached_invalid_recovers_from_remote() -> Res {
        let (paths, _temp_dir) = DomainPaths::from_temp_dir()?;
        let storage = LocalStorage::new();

        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "stale_hash".to_string(),
            origin: None,
        };

        // Write invalid data to simulate a stale Parquet cache
        let cache_path = paths.cached_manifest(&manifest_uri);
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;
        storage
            .write_byte_stream(&cache_path, ByteStream::from_static(b"PAR1_invalid"))
            .await?;

        // Set up valid JSONL on the remote
        let jsonl = storage.read_bytes(fixtures::manifest::path()?).await?;
        let remote = MockRemote::default();
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/{}",
            manifest_uri.bucket, manifest_uri.hash
        ))?;
        remote
            .put_object(&manifest_uri.origin, &remote_uri, jsonl)
            .await?;

        // Should recover: delete stale cache, re-fetch, succeed
        let manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest_uri).await?;

        assert_eq!(manifest.header.message, Some("Initial".to_string()));
        assert_eq!(manifest.rows.len(), 10);
        assert!(manifest.get_record(&PathBuf::from("e0-0.txt")).is_some());

        // Verify the cache file was replaced with valid JSONL
        let cached_bytes = storage.read_bytes(&cache_path).await?;
        let first_line = std::str::from_utf8(&cached_bytes)
            .expect("cached file should be valid UTF-8")
            .lines()
            .next()
            .expect("cached file should not be empty");
        serde_json::from_str::<serde_json::Value>(first_line)
            .expect("cached file should contain valid JSON");

        // Verify the cached file can be loaded as a manifest
        let reloaded = Manifest::from_path(&storage, &cache_path).await?;
        assert_eq!(reloaded.rows.len(), 10);

        Ok(())
    }

    /// Verifies that when a manifest is not cached,
    /// and the manifest exists remotely in JSONL format,
    /// it is downloaded and cached directly in JSONL format.
    #[test(tokio::test)]
    async fn test_caching_jsonl() -> Res {
        let paths = DomainPaths::default();

        // Define the manifest URI to simulate a package lookup.
        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "deadbeef".to_string(), // we use it for manifest location, we don't verify the hash
            origin: None,
        };

        // Simulate the remote JSONL manifest.
        // The JSONL data is loaded from a mocked fixture and placed in the remote location.
        let jsonl = std::fs::read(fixtures::manifest::path()?)?;
        let remote = MockRemote::default();
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/{}",
            manifest_uri.bucket, manifest_uri.hash
        ))?;
        remote
            .put_object(&manifest_uri.origin, &remote_uri, jsonl.clone())
            .await?;

        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        // Fetch the manifest from the remote location.
        // This should cache the JSONL manifest locally in the same format.
        let cached_manifest =
            cache_remote_manifest(&paths, &storage, &remote, &manifest_uri).await?;

        // Verify that the JSONL manifest is cached locally.
        let cache_path = PathBuf::from(format!(
            ".quilt/packages/{}/{}",
            manifest_uri.bucket, manifest_uri.hash
        ));
        assert!(storage.exists(cache_path).await);

        // Verify that the cached manifest contains valid records
        assert!(cached_manifest
            .get_record(&PathBuf::from("e0-0.txt"))
            .is_some());

        Ok(())
    }
}
