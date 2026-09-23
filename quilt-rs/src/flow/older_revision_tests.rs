//! `qhq-a4za`: install an older revision, download every file, and a file the
//! newer revision modifies must not read as changed. Entry status is measured
//! against the installed revision, never against `latest`.

use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use aws_sdk_s3::primitives::ByteStream;
use test_log::test;

use crate::Error;
use crate::InstallPathError;
use crate::Res;
use crate::checksum::calculate_hash;
use crate::flow;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::lineage::UpstreamState;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths;
use crate::paths::DomainPaths;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

const BUCKET: &str = "b";

/// A published row for `body` at `logical_key`, with its object put on the
/// remote. `Some(version)` pins the key to that S3 version, the way a push to a
/// versioned bucket records it; `None` is a bare key, as an unversioned bucket
/// leaves it — so a later revision's put at the same key replaces the bytes.
async fn publish_row(
    remote: &MockRemote,
    scratch: &Path,
    host_config: &HostConfig,
    logical_key: &str,
    version: Option<&str>,
    body: &[u8],
) -> Res<ManifestRow> {
    let storage = LocalStorage::new();
    let local = scratch.join(format!("{logical_key}.{}", uuid::Uuid::new_v4()));
    storage
        .write_byte_stream(&local, ByteStream::from(body.to_vec()))
        .await?;
    let row = calculate_hash(&storage, &local, Path::new(logical_key), host_config).await?;
    let physical_key = match version {
        Some(version) => format!("s3://{BUCKET}/f/a/{logical_key}?versionId={version}"),
        None => format!("s3://{BUCKET}/f/a/{logical_key}"),
    };
    remote
        .put_object(None, &S3Uri::from_str(&physical_key)?, body.to_vec())
        .await?;
    Ok(ManifestRow {
        physical_key,
        ..row
    })
}

async fn publish_manifest(remote: &MockRemote, hash: &str, rows: Vec<ManifestRow>) -> Res {
    let manifest = Manifest {
        rows,
        ..Manifest::default()
    };
    remote
        .put_object(
            None,
            &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/packages/{hash}"))?,
            ByteStream::from(&manifest),
        )
        .await
}

/// Revision 1 installed, nothing downloaded yet. Latest is revision 2, which
/// modifies `changes.txt` and keeps `same.txt`.
struct Rev1Installed {
    storage: LocalStorage,
    remote: MockRemote,
    domain_paths: DomainPaths,
    namespace: Namespace,
    package_home: PathBuf,
    lineage: PackageLineage,
    _dirs: [tempfile::TempDir; 3],
}

impl Rev1Installed {
    async fn new(versioned: bool) -> Res<Self> {
        let v = |version| versioned.then_some(version);
        let host_config = HostConfig::default();
        let scratch = tempfile::TempDir::new()?;
        let remote = MockRemote::default();
        let namespace: Namespace = ("f", "a").into();

        let same = publish_row(
            &remote,
            scratch.path(),
            &host_config,
            "same.txt",
            v("v1"),
            b"same",
        )
        .await?;
        let old = publish_row(
            &remote,
            scratch.path(),
            &host_config,
            "changes.txt",
            v("v1"),
            b"one",
        )
        .await?;
        let new = publish_row(
            &remote,
            scratch.path(),
            &host_config,
            "changes.txt",
            v("v2"),
            b"two",
        )
        .await?;
        publish_manifest(&remote, "REV1", vec![old, same.clone()]).await?;
        publish_manifest(&remote, "REV2", vec![new, same]).await?;
        remote
            .put_object(
                None,
                &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/named_packages/f/a/latest"))?,
                b"REV2".to_vec(),
            )
            .await?;

        let storage = LocalStorage::new();
        let (domain_lineage, home_dir) = DomainLineage::from_temp_dir()?;
        let home = domain_lineage.home.clone();
        let (domain_paths, paths_dir) = DomainPaths::from_temp_dir()?;

        // Revision 1 explicitly — not latest.
        let rev1 = ManifestUri {
            bucket: BUCKET.to_string(),
            namespace: namespace.clone(),
            hash: "REV1".to_string(),
            origin: None,
        };
        let domain_lineage =
            flow::install_package(domain_lineage, &domain_paths, &storage, &remote, &rev1).await?;
        let lineage = domain_lineage.packages[&namespace].clone();
        assert_eq!(lineage.latest_hash, "REV2");

        Ok(Self {
            package_home: paths::package_home(&home, &namespace),
            storage,
            remote,
            domain_paths,
            namespace,
            lineage,
            _dirs: [scratch, home_dir, paths_dir],
        })
    }

    fn installed_manifest(&self) -> PathBuf {
        self.domain_paths
            .installed_manifest(&self.namespace, "REV1")
    }

    /// Select all + Download.
    async fn download_all(&self) -> Res<PackageLineage> {
        let mut manifest = Manifest::from_path(&self.storage, &self.installed_manifest()).await?;
        let all = [PathBuf::from("changes.txt"), PathBuf::from("same.txt")];
        flow::install_paths(
            self.lineage.clone(),
            &mut manifest,
            &self.domain_paths,
            self.package_home.clone(),
            self.namespace.clone(),
            &self.storage,
            &self.remote,
            &all.iter().collect::<Vec<_>>(),
        )
        .await
    }

    async fn status(&self, lineage: PackageLineage) -> Res<InstalledPackageStatus> {
        let manifest = Manifest::from_path(&self.storage, &self.installed_manifest()).await?;
        let (_, status) = flow::status(
            lineage,
            &self.storage,
            &manifest,
            &self.package_home,
            HostConfig::default(),
        )
        .await?;
        Ok(status)
    }
}

/// The control: rows pinned to S3 versions fetch revision 1's own bytes, and
/// nothing reads as changed.
#[test(tokio::test)]
async fn downloading_an_older_revision_leaves_untouched_files_unchanged() -> Res {
    let package = Rev1Installed::new(true).await?;
    let lineage = package.download_all().await?;

    let on_disk = tokio::fs::read(package.package_home.join("changes.txt")).await?;
    assert_eq!(on_disk, b"one");
    let status = package.status(lineage).await?;
    assert_eq!(status.upstream_state, UpstreamState::Behind);
    assert!(
        status.changes.is_empty(),
        "untouched files read as changed: {:?}",
        status.changes.keys().collect::<Vec<_>>()
    );
    Ok(())
}

/// `qhq-a4za`. A bare physical key names whatever the object is *now*. Once
/// revision 2 has replaced it, downloading revision 1 used to fetch revision
/// 2's bytes and file them under revision 1's row, and the untouched file read
/// Modified. Bytes that do not match their row are refused, the working tree
/// is left as it was, and the refusal holds on retry: the wrong bytes must not
/// have been cached under revision 1's hash either.
#[test(tokio::test)]
async fn an_unversioned_key_never_places_latests_bytes_under_an_older_row() -> Res {
    let package = Rev1Installed::new(false).await?;

    for attempt in ["first", "retry"] {
        match package.download_all().await {
            Err(Error::InstallPath(InstallPathError::ContentMismatch(path))) => {
                assert_eq!(path, PathBuf::from("changes.txt"), "{attempt}");
            }
            Err(err) => panic!("{attempt}: refused for the wrong reason: {err}"),
            Ok(lineage) => panic!(
                "{attempt}: placed {:?} under revision 1's row; status changes: {:?}",
                String::from_utf8_lossy(
                    &tokio::fs::read(package.package_home.join("changes.txt")).await?
                ),
                package
                    .status(lineage)
                    .await?
                    .changes
                    .keys()
                    .collect::<Vec<_>>()
            ),
        }
        assert!(
            !package
                .storage
                .exists(package.package_home.join("changes.txt"))
                .await
        );
        assert!(
            !package
                .storage
                .exists(package.package_home.join("same.txt"))
                .await
        );
    }
    Ok(())
}
