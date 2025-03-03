use tokio::io::AsyncReadExt;
use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Header;
use crate::manifest::Manifest;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::ManifestUriLegacy;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

async fn stream_jsonl_rows(jsonl: Manifest) -> impl RowsStream {
    tokio_stream::iter(jsonl.rows)
        .map(Row::try_from)
        .map(|rows| Ok(vec![rows]))
}

async fn is_parquet(remote: &impl Remote, manifest: &ManifestUri) -> Res<bool> {
    remote
        .exists(&manifest.catalog, &S3Uri::from(manifest))
        .await
}

async fn fetch_parquet(remote: &impl Remote, manifest: &ManifestUri) -> Res<Vec<u8>> {
    let s3_uri = S3Uri::from(manifest);
    let mut contents = remote.get_object(&manifest.catalog, &s3_uri).await?;
    let mut output = Vec::new();
    contents.read_to_end(&mut output).await?;
    Ok(output)
}

async fn fetch_jsonl(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res<Manifest> {
    let s3_uri: S3Uri = ManifestUriLegacy::from(manifest_uri).into();
    let contents = remote.get_object(&manifest_uri.catalog, &s3_uri).await?;
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
    info!("⏳ Caching remote manifest: {}", manifest_uri.display());

    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    // let manifest_uri = resolve_manifest_uri(remote, uri).await?;
    let cache_dir = paths.manifest_cache_dir(&manifest_uri.bucket);
    let cache_path = cache_dir.join(&manifest_uri.hash);

    let mut manifest_path = cache_path.clone();

    if !storage.exists(&cache_path).await {
        debug!("🔍 Manifest does not exist in cache, fetching from remote");
        // Does not exist yet
        if is_parquet(remote, manifest_uri).await? {
            debug!(
                "⏳ Manifest {} stored remotely in Parquet format. Fetching…",
                manifest_uri.display()
            );
            let manifest = fetch_parquet(remote, manifest_uri).await?;
            debug!("✔️ Fetched manifest. Size: {}", manifest.len());
            storage.write_file(&cache_path, &manifest).await?;
            debug!("✔️ Manifest has written to {}", cache_path.display());
        } else {
            debug!(
                "⏳ Manifest {} stored remotely in JSONL format. Fetching…",
                manifest_uri.display()
            );
            let manifest = fetch_jsonl(remote, manifest_uri).await?;
            debug!("✔️ Fetched JSONL manifest");
            let header = Header::from(&manifest);
            let stream = stream_jsonl_rows(manifest).await;
            let (dest_path, top_hash) =
                build_manifest_from_rows_stream(storage, cache_dir, header, stream).await?;
            if top_hash != manifest_uri.hash {
                return Err(Error::ManifestPath(format!(
                    "Top hash mismatch: expected {}, got {}",
                    manifest_uri.hash, top_hash
                ))
                .into());
            }
            debug!(
                "✔️ Manifest has converted to Parquet and written to {}",
                dest_path.display()
            );
            manifest_path = dest_path
        };
    } else {
        debug!("✔️ Manifest exists already in {}", cache_path.display());
    }

    info!("✔️ Manifest {} was written …", manifest_uri.display());

    let manifest = Table::read_from_path(storage, &manifest_path).await?;

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
) -> Res<Table> {
    cache_remote_manifest(paths, storage, remote, manifest_uri).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::str::FromStr;

    use test_log::test;

    use crate::fixtures;
    use crate::paths::scaffold_paths;

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
            hash: "c".to_string(),
            catalog: None,
        };
        let cache_path = paths.manifest_cache(&manifest_uri.bucket, &manifest_uri.hash);

        // Prepare the reference manifest file.
        // It is copied into the cache path to simulate a cached manifest.
        let parquet = std::fs::read(fixtures::manifest::parquet())?;
        let storage = fixtures::storage::MockStorage::default();
        storage.write_file(&cache_path, &parquet).await?;

        // Although there is no direct assertion for `remote.expect_get_object().never()`,
        // we know the remote is not called because a missing key would throw an error.
        let remote = fixtures::remote::MockRemote::default();

        // Since the manifest is cached, it should be retrieved from the cache
        // without any remote interaction.
        let cached_manifest =
            cache_remote_manifest(&paths, &storage, &remote, &manifest_uri).await?;

        // Verify that the cached manifest matches the reference manifest
        assert_eq!(
            cached_manifest.header.info.get("message").unwrap(),
            "test_spec_write 2023-11-29T14:01:39.543975"
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
            hash: "c".to_string(),
            catalog: None,
        };
        let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);
        let storage = fixtures::storage::MockStorage::default();
        storage.write_file(cache_path, &Vec::new()).await?;

        let remote = fixtures::remote::MockRemote::default();

        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await;

        assert_eq!(
            cached_manifest.unwrap_err().to_string(),
            "Parquet error: External: Invalid argument (os error 22)"
        );

        Ok(())
    }

    /// Verifies that when a manifest is not cached,
    /// and the manifest exists remotely in a "parquet" location (with the "1220" prefix),
    /// it is downloaded and cached correctly.
    #[test(tokio::test)]
    async fn test_caching_parquet() -> Res {
        let paths = DomainPaths::default();

        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "c".to_string(),
            catalog: None,
        };

        // Simulate the existence of a reference manifest remotely.
        // This is done by "writing" the reference manifest to a mocked remote location.
        // Technically, it is written to a temporary directory with an URI as a path.
        let parquet = std::fs::read(fixtures::manifest::parquet())?;
        let remote = fixtures::remote::MockRemote::default();
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/1220{}.parquet",
            manifest.bucket, manifest.hash
        ))?;
        remote
            .put_object(&manifest.catalog, &remote_uri, parquet.clone())
            .await?;

        let storage = fixtures::storage::MockStorage::default();

        // Fetch the manifest from the remote location.
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await?;
        assert_eq!(
            cached_manifest.header.info.get("message").unwrap(),
            "test_spec_write 2023-11-29T14:01:39.543975"
        );

        // Manifest should be cached locally after being retrieved at the correct local path.
        let cache_path = PathBuf::from(format!(
            ".quilt/packages/{}/{}",
            manifest.bucket, manifest.hash
        ));
        assert_eq!(storage.read_file(cache_path).await?, parquet);

        Ok(())
    }

    /// Verifies that when a manifest is not cached,
    /// and the manifest exists remotely in JSONL format,
    /// it is downloaded, converted to Parquet, and cached correctly.
    #[test(tokio::test)]
    async fn test_caching_jsonl() -> Res {
        let paths = DomainPaths::default();

        // Define the manifest URI to simulate a package lookup.
        let manifest = ManifestUri {
            bucket: "a".to_string(),
            namespace: ("f", "b").into(),
            hash: "3af08e839fec032c6804596d32932f6f0550abe8b9696c56ed15fe7f8e853ebd".to_string(),
            catalog: None,
        };

        // Simulate the remote JSONL manifest.
        // The JSONL data is loaded from a mocked fixture and placed in the remote location.
        let jsonl = std::fs::read(fixtures::manifest::jsonl())?;
        let remote = fixtures::remote::MockRemote::default();
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/{}",
            manifest.bucket, manifest.hash
        ))?;
        remote
            .put_object(&manifest.catalog, &remote_uri, jsonl.clone())
            .await?;

        let storage = fixtures::storage::MockStorage::default();
        scaffold_paths(&storage, paths.required_for_caching(&manifest.bucket)).await?;

        // Fetch the manifest from the remote location.
        // This should trigger a conversion from JSONL to Parquet and cache the result locally.
        let cached_manifest = cache_remote_manifest(&paths, &storage, &remote, &manifest).await?;

        // Verify that the converted Parquet manifest is cached locally.
        let cache_path = PathBuf::from(format!(
            ".quilt/packages/{}/{}",
            manifest.bucket, manifest.hash
        ));
        assert!(storage.exists(cache_path).await);

        // Verify that the cached manifest contains valid records
        assert!(cached_manifest
            .get_record(&PathBuf::from("README.md"))
            .await?
            .is_some());

        Ok(())
    }
}
