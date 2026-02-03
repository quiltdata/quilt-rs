use tracing::debug;
use tracing::info;

use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::S3Uri;
use crate::Res;

async fn fetch_jsonl(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res<Manifest> {
    let s3_uri: S3Uri = manifest_uri.clone().into();
    let contents = remote
        .get_object_stream(&manifest_uri.origin, &s3_uri)
        .await?;
    Manifest::from_reader(contents.body.into_async_read()).await
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
    let cache_dir = paths.manifest_cache_dir(&manifest_uri.bucket);
    let cache_path = cache_dir.join(&manifest_uri.hash);

    let manifest_path = cache_path.clone();

    if !storage.exists(&cache_path).await {
        debug!("🔍 Manifest does not exist in cache, fetching from remote");
        debug!(
            "⏳ Fetching JSONL manifest {} from remote…",
            manifest_uri.display()
        );
        let manifest = fetch_jsonl(remote, manifest_uri).await?;
        debug!("✔️ Fetched JSONL manifest");

        // Write JSONL to cache
        let jsonl_content = manifest.to_jsonlines();
        storage
            .write_file(&cache_path, jsonl_content.as_bytes())
            .await?;
        debug!("✔️ JSONL manifest written to {}", cache_path.display());
    } else {
        debug!("✔️ Manifest exists already in {}", cache_path.display());
    }

    info!("✔️ Manifest {} was written …", manifest_uri.display());

    let manifest = Manifest::from_path(storage, &manifest_path).await?;

    info!("✔️ … and, Successfully cached:\n{:?}", manifest.header);

    Ok(manifest)
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

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;

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
            hash: fixtures::manifest::JSONL_HASH.to_string(),
            origin: None,
        };
        let cache_path = paths.manifest_cache(&manifest_uri.bucket, &manifest_uri.hash);

        // Prepare the reference manifest file.
        // It is copied into the cache path to simulate a cached manifest.
        let jsonl = std::fs::read(fixtures::manifest::jsonl()?)?;
        let storage = MockStorage::default();
        storage.write_file(&cache_path, &jsonl).await?;

        // Although there is no direct assertion for `remote.expect_get_object().never()`,
        // we know the remote is not called because a missing key would throw an error.
        let remote = MockRemote::default();

        // Since the manifest is cached, it should be retrieved from the cache
        // without any remote interaction.
        let cached_manifest =
            cache_remote_manifest(&paths, &storage, &remote, &manifest_uri).await?;

        // Verify that the cached manifest matches the reference manifest
        // JSONL fixture has no message but has user_meta set to null
        assert_eq!(
            cached_manifest.header.user_meta,
            Some(serde_json::Value::Null)
        );

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
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        let storage = MockStorage::default();
        storage.write_file(cache_path, &Vec::new()).await?;

        let remote = MockRemote::default();

        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await;

        // Since we now use JSONL format, the error message will be different
        assert!(cached_manifest.is_err());

        Ok(())
    }

    /// Verifies that when a manifest is not cached,
    /// and the manifest exists remotely in JSONL format,
    /// it is downloaded and cached directly in JSONL format.
    #[test(tokio::test)]
    async fn test_caching_jsonl() -> Res {
        let paths = DomainPaths::default();

        // Define the manifest URI to simulate a package lookup.
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: fixtures::manifest::JSONL_HASH.to_string(),
            origin: None,
        };

        // Simulate the remote JSONL manifest.
        // The JSONL data is loaded from a mocked fixture and placed in the remote location.
        let jsonl = std::fs::read(fixtures::manifest::jsonl()?)?;
        let remote = MockRemote::default();
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/{}",
            manifest.bucket, manifest.hash
        ))?;
        remote
            .put_object(&manifest.origin, &remote_uri, jsonl.clone())
            .await?;

        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest.bucket)
            .await?;

        // Fetch the manifest from the remote location.
        // This should cache the JSONL manifest locally in the same format.
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await?;

        // Verify that the JSONL manifest is cached locally.
        let cache_path = PathBuf::from(format!(
            ".quilt/packages/{}/{}",
            manifest.bucket, manifest.hash
        ));
        assert!(storage.exists(cache_path).await);

        // Verify that the cached manifest contains valid records
        assert!(cached_manifest
            .get_record(&PathBuf::from("README.md"))
            .is_some());

        Ok(())
    }
}
