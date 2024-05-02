use std::path::PathBuf;

use tracing::log;

use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Table;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
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
    manifest_uri
        .upload_from(storage, remote, &cache_path)
        .await?;

    // Upload a quilt3 manifest for backward compatibility.
    manifest_uri.upload_legacy(remote, &manifest).await?;
    log::debug!("Uploaded remote manifest: {:?}", manifest_uri);
    Ok(manifest_uri)
}
