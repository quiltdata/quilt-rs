use std::collections::BTreeMap;

use arrow::error::ArrowError;
use aws_sdk_s3::error::SdkError;
use multihash::Multihash;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::paths;
use crate::quilt::manifest;
use crate::quilt::manifest_handle::{CachedManifest, ReadableManifest, RemoteManifest};
use crate::quilt::storage;
use crate::quilt::Error;
use crate::{quilt4::table::HEADER_ROW, Row4, Table, UPath};

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

// FIMXE: CachedManifest::browse(&RemoteManifest)
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

        let result = client
            .get_object()
            .bucket(&manifest.bucket)
            .key(paths::get_manifest_key(&manifest.hash))
            .send()
            .await;

        match result {
            Ok(output) => {
                let mut contents = Vec::new();
                output
                    .body
                    .into_async_read()
                    .read_to_end(&mut contents)
                    .await?;
                storage::fs::write(&cache_path, &contents).await?;
            }
            Err(SdkError::ServiceError(err)) if err.err().is_no_such_key() => {
                // Fallback: Download the JSONL manifest.
                let result = client
                    .get_object()
                    .bucket(&manifest.bucket)
                    .key(paths::get_manifest_key(&manifest.hash))
                    .send()
                    .await
                    .map_err(|err| Error::S3(err.to_string()))?;

                let quilt3_manifest =
                    manifest::Manifest::from_file(result.body.into_async_read()).await?;
                let header = Row4 {
                    name: HEADER_ROW.into(),
                    place: HEADER_ROW.into(),
                    path: None,
                    size: 0,
                    hash: Multihash::default(),
                    info: serde_json::json!({
                        "message": quilt3_manifest.header.message,
                        "version": quilt3_manifest.header.version,
                    }),
                    meta: match quilt3_manifest.header.user_meta {
                        Some(meta) => meta.into(),
                        None => serde_json::Value::Null,
                    },
                };
                let mut records = BTreeMap::new();
                for row in quilt3_manifest.rows {
                    let mut info = row.meta.unwrap_or_default();
                    let meta = info.remove("user_meta").unwrap_or_default();
                    records.insert(
                        row.logical_key.clone(),
                        Row4 {
                            name: row.logical_key,
                            place: row.physical_key,
                            path: None,
                            size: row.size,
                            hash: row.hash.try_into()?,
                            info: info.into(),
                            meta,
                        },
                    );
                }
                let table = Table { header, records };
                table.write_to_upath(&UPath::Local(cache_path)).await?
            }
            Err(err) => {
                return Err(Error::S3(err.to_string()));
            }
        }
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
