//! Install an older revision, download every file, and a file the newer
//! revision modifies must not read as changed. Entry status is measured against
//! the installed revision, never against `latest`.
//!
//! Real packages pin every key to an S3 version. The unversioned case is a
//! bucket without versioning, where a later revision's put replaces the bytes
//! an older row names.

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
use crate::flow::InstallPathsReport;
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
        Self::with_versions(versioned.then_some("v1"), versioned.then_some("v2")).await
    }

    /// Revision 1's keys pinned to `v1`, and revision 2's `changes.txt` to `v2`.
    async fn with_versions(v1: Option<&str>, v2: Option<&str>) -> Res<Self> {
        let host_config = HostConfig::default();
        let scratch = tempfile::TempDir::new()?;
        let remote = MockRemote::default();
        let namespace: Namespace = ("f", "a").into();

        let same = publish_row(
            &remote,
            scratch.path(),
            &host_config,
            "same.txt",
            v1,
            b"same",
        )
        .await?;
        let old = publish_row(
            &remote,
            scratch.path(),
            &host_config,
            "changes.txt",
            v1,
            b"one",
        )
        .await?;
        let new = publish_row(
            &remote,
            scratch.path(),
            &host_config,
            "changes.txt",
            v2,
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
    async fn download_all(&self) -> Res<(PackageLineage, InstallPathsReport)> {
        let all = [PathBuf::from("changes.txt"), PathBuf::from("same.txt")];
        self.download(self.lineage.clone(), &all).await
    }

    async fn download(
        &self,
        lineage: PackageLineage,
        paths: &[PathBuf],
    ) -> Res<(PackageLineage, InstallPathsReport)> {
        let mut manifest = Manifest::from_path(&self.storage, &self.installed_manifest()).await?;
        flow::install_paths(
            lineage,
            &mut manifest,
            &self.domain_paths,
            self.package_home.clone(),
            self.namespace.clone(),
            &self.storage,
            &self.remote,
            &paths.iter().collect::<Vec<_>>(),
        )
        .await
    }

    /// Replaces the bytes at the exact version revision 1's `changes.txt` row
    /// names, so the row's recorded hash no longer describes them.
    async fn put_other_bytes_at_changes_v1(&self) -> Res {
        self.remote
            .put_object(
                None,
                &S3Uri::from_str(&format!("s3://{BUCKET}/f/a/changes.txt?versionId=v1"))?,
                b"not one".to_vec(),
            )
            .await
    }

    async fn status(&self, lineage: PackageLineage) -> Res<InstalledPackageStatus> {
        let manifest = Manifest::from_path(&self.storage, &self.installed_manifest()).await?;
        let (_, status) = flow::status(
            lineage,
            &self.storage,
            &self.domain_paths,
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
    let (lineage, report) = package.download_all().await?;
    assert!(report.skipped.is_empty(), "skipped {:?}", report.skipped);

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

/// A bare physical key names whatever the object is *now*. Once revision 2 has
/// replaced it, downloading revision 1 used to fetch revision 2's bytes and file
/// them under revision 1's row, and the untouched file read Modified. Now the
/// row whose bytes are gone is skipped and reported, and every other row is
/// installed: an unversioned bucket degrades, it does not fail the download.
#[test(tokio::test)]
async fn an_unversioned_key_skips_the_replaced_row_and_installs_the_rest() -> Res {
    let package = Rev1Installed::new(false).await?;

    let (lineage, report) = package.download_all().await?;

    assert_eq!(report.skipped, vec![PathBuf::from("changes.txt")]);
    assert_eq!(report.hash_mismatch, Vec::<PathBuf>::new());
    assert_eq!(
        tokio::fs::read(package.package_home.join("same.txt")).await?,
        b"same"
    );
    assert!(
        !package
            .storage
            .exists(package.package_home.join("changes.txt"))
            .await,
        "revision 2's bytes were placed under revision 1's row"
    );
    assert_eq!(
        lineage.paths.keys().collect::<Vec<_>>(),
        vec![&PathBuf::from("same.txt")]
    );
    let status = package.status(lineage).await?;
    assert!(
        status.changes.is_empty(),
        "untouched files read as changed: {:?}",
        status.changes.keys().collect::<Vec<_>>()
    );
    Ok(())
}

/// Asking again for what was skipped still skips it: the wrong bytes were never
/// filed under revision 1's hash, so a retry fetches afresh rather than taking
/// them from the object store as a cache hit.
#[test(tokio::test)]
async fn retrying_a_skipped_path_does_not_take_wrong_bytes_from_the_store() -> Res {
    let package = Rev1Installed::new(false).await?;
    let (lineage, report) = package.download_all().await?;

    let (lineage, report) = package.download(lineage, &report.skipped).await?;

    assert_eq!(report.skipped, vec![PathBuf::from("changes.txt")]);
    assert!(
        !package
            .storage
            .exists(package.package_home.join("changes.txt"))
            .await,
        "the retry placed revision 2's bytes under revision 1's row"
    );
    let manifest = Manifest::from_path(&package.storage, &package.installed_manifest()).await?;
    let row = manifest
        .get_record(&PathBuf::from("changes.txt"))
        .expect("revision 1 has changes.txt");
    assert!(
        !package
            .storage
            .exists(package.domain_paths.object(row.hash.digest()))
            .await,
        "revision 2's bytes are in the store under revision 1's hash"
    );
    assert!(package.status(lineage).await?.changes.is_empty());
    Ok(())
}

/// A key pinned to its version gets exactly the bytes that were pushed, so when
/// they do not hash to the row, the row's hash is wrong, not the bucket's
/// content. The row is left out like a skip, but reported apart from one.
#[test(tokio::test)]
async fn a_versioned_row_whose_hash_is_wrong_is_reported_as_a_hash_mismatch() -> Res {
    let package = Rev1Installed::new(true).await?;
    package.put_other_bytes_at_changes_v1().await?;

    let (lineage, report) = package.download_all().await?;

    assert_eq!(report.hash_mismatch, vec![PathBuf::from("changes.txt")]);
    assert!(report.skipped.is_empty(), "skipped {:?}", report.skipped);
    assert_eq!(
        tokio::fs::read(package.package_home.join("same.txt")).await?,
        b"same"
    );
    assert!(
        !package
            .storage
            .exists(package.package_home.join("changes.txt"))
            .await
    );
    assert_eq!(
        lineage.paths.keys().collect::<Vec<_>>(),
        vec![&PathBuf::from("same.txt")]
    );
    let manifest = Manifest::from_path(&package.storage, &package.installed_manifest()).await?;
    let row = manifest
        .get_record(&PathBuf::from("changes.txt"))
        .expect("revision 1 has changes.txt");
    assert!(
        !package
            .storage
            .exists(package.domain_paths.object(row.hash.digest()))
            .await,
        "bytes that do not match the row are in the store under its hash"
    );
    Ok(())
}

/// The reconcile cannot leave a row out, so a wrong hash on a versioned row
/// refuses it, and says the package is at fault rather than the bucket.
#[test(tokio::test)]
async fn the_reconcile_refuses_a_versioned_row_whose_hash_is_wrong() -> Res {
    let package = Rev1Installed::new(true).await?;
    package.put_other_bytes_at_changes_v1().await?;
    let mut manifest = Manifest::from_path(&package.storage, &package.installed_manifest()).await?;
    let changes = PathBuf::from("changes.txt");

    let result = flow::install_paths_over(
        package.lineage.clone(),
        &mut manifest,
        &package.domain_paths,
        package.package_home.clone(),
        package.namespace.clone(),
        &package.storage,
        &package.remote,
        &[&changes],
        &flow::Protect::Nothing,
        flow::OnMismatch::Refuse,
    )
    .await;

    assert!(
        matches!(
            &result,
            Err(Error::InstallPath(InstallPathError::HashMismatch(path))) if path == &changes
        ),
        "{result:?}"
    );
    Ok(())
}

/// While versioning is suspended S3 writes the version `null`, and a later put
/// overwrites that same version, so a row pinned to it can be replaced like a
/// bare key: skipped, not a wrong hash.
#[test(tokio::test)]
async fn a_row_pinned_to_the_null_version_skips_the_replaced_row() -> Res {
    let package = Rev1Installed::with_versions(Some("null"), Some("null")).await?;

    let (_, report) = package.download_all().await?;

    assert_eq!(report.skipped, vec![PathBuf::from("changes.txt")]);
    assert_eq!(report.hash_mismatch, Vec::<PathBuf>::new());
    Ok(())
}
