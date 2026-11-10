//! Status when the catalog host can't be reached.

use std::future::Future;

use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::error::RemoteCatalogError;
use crate::local_domain::LocalDomain;
use crate::object_hash::MULTIHASH_CRC64_NVME;
use crate::object_hash::ObjectHash;
use quilt_uri::fixtures;

/// A remote whose every request fails before an answer, as with no network.
struct UnreachableRemote;

fn no_network() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "no network",
    ))
}

impl crate::io::remote::Remote for UnreachableRemote {
    fn exists(&self, _host: Option<&Host>, _s3_uri: &S3Uri) -> impl Future<Output = Res<bool>> {
        std::future::ready(Err(no_network()))
    }
    fn get_object_stream(
        &self,
        _host: Option<&Host>,
        _s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<crate::io::remote::RemoteObjectStream>> {
        std::future::ready(Err(no_network()))
    }
    fn resolve_url(
        &self,
        _host: Option<&Host>,
        _s3_uri: &S3Uri,
    ) -> impl Future<Output = Res<S3Uri>> {
        std::future::ready(Err(no_network()))
    }
    fn put_object(
        &self,
        _host: Option<&Host>,
        _s3_uri: &S3Uri,
        _contents: impl Into<aws_sdk_s3::primitives::ByteStream>,
    ) -> impl Future<Output = Res> {
        std::future::ready(Err(no_network()))
    }
    fn upload_file(
        &self,
        _host_config: &HostConfig,
        _source_path: impl AsRef<std::path::Path>,
        _dest_uri: &S3Uri,
        _size: u64,
    ) -> impl Future<Output = Res<(S3Uri, ObjectHash)>> {
        std::future::ready(Err(no_network()))
    }
    fn host_config(&self, host: Option<&Host>) -> impl Future<Output = Res<HostConfig>> {
        std::future::ready(Err(Error::RemoteCatalog(RemoteCatalogError::HostConfig(
            format!("Failed to fetch config from {host:?}: no network"),
        ))))
    }
    fn verify_bucket(&self, _bucket: &str) -> impl Future<Output = Res> {
        std::future::ready(Err(no_network()))
    }
    fn published_revisions(
        &self,
        _host: Option<&Host>,
        _bucket: &str,
        _namespace: &Namespace,
    ) -> impl Future<Output = Res<Vec<String>>> {
        std::future::ready(Err(no_network()))
    }
}

/// A package committed with CRC64 hashes, installed from a catalog host that
/// now can't be reached, with one new file in its folder.
async fn unreachable_package(
    temp_dir: &TempDir,
) -> Res<InstalledPackage<LocalStorage, UnreachableRemote>> {
    let domain = LocalDomain::new(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;
    let namespace: Namespace = ("demo", "offline").into();
    let (package, _) = domain
        .create_package(namespace.clone(), None, Some("empty".to_string()))
        .await?;
    let home = package.package_home().await?;
    std::fs::write(home.join("kept.txt"), b"kept")?;
    let committed = package
        .commit(
            "add".to_string(),
            UserMeta::Keep,
            None,
            Some(HostConfig::default_crc64()),
        )
        .await?;
    let hash = committed.hash().expect("committed").to_string();
    package
        .lineage
        .edit(&package.storage, |lineage| {
            lineage.commit = None;
            lineage.remote_uri = Some(ManifestUri {
                origin: Some(fixtures::host()),
                bucket: "bkt".to_string(),
                namespace: namespace.clone(),
                hash: hash.clone(),
            });
            lineage.base_hash.clone_from(&hash);
            lineage.latest_hash.clone_from(&hash);
        })
        .await?;
    std::fs::write(home.join("new.txt"), b"new")?;
    Ok(InstalledPackage {
        lineage: package.lineage.clone(),
        paths: package.paths.clone(),
        remote: Arc::new(UnreachableRemote),
        storage: package.storage.clone(),
        namespace,
        state_store: None,
    })
}

fn added_algorithm(status: &lineage::InstalledPackageStatus) -> u64 {
    let Some(lineage::Change::Added(row)) = status.changes.get(Path::new("new.txt")) else {
        panic!("new.txt is not added: {:?}", status.changes);
    };
    row.hash.algorithm()
}

/// Status names no object, so it does not need the host's config: it hashes
/// with the algorithm the manifest already uses and says the tip was not read.
#[test(tokio::test)]
async fn status_reads_the_local_package_when_the_host_cannot_be_reached() -> Res {
    let temp_dir = TempDir::new()?;
    let package = unreachable_package(&temp_dir).await?;

    let status = package.status(None).await?;

    assert!(!status.latest_refreshed, "the tip was not read");
    assert_eq!(status.upstream_state, lineage::UpstreamState::UpToDate);
    assert_eq!(status.changes.len(), 1, "{:?}", status.changes);
    assert_eq!(added_algorithm(&status), MULTIHASH_CRC64_NVME);
    Ok(())
}

/// The file pane's local-only read takes the same fallback.
#[test(tokio::test)]
async fn a_local_status_does_not_need_the_host_either() -> Res {
    let temp_dir = TempDir::new()?;
    let package = unreachable_package(&temp_dir).await?;

    let status = package.recompute_local_status(None).await?;

    assert_eq!(added_algorithm(&status), MULTIHASH_CRC64_NVME);
    Ok(())
}

/// A commit names its objects, so it still needs the host's answer.
#[test(tokio::test)]
async fn a_commit_still_needs_the_host() -> Res {
    let temp_dir = TempDir::new()?;
    let package = unreachable_package(&temp_dir).await?;

    let result = package
        .commit("offline".to_string(), UserMeta::Keep, None, None)
        .await;

    assert!(
        matches!(
            result,
            Err(Error::RemoteCatalog(RemoteCatalogError::HostConfig(_)))
        ),
        "{result:?}"
    );
    Ok(())
}
