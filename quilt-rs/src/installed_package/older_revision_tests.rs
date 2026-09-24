//! `qhq-a4za`: install an older revision, download every file, and a file the
//! newer revision modifies must not read as changed.
//!
//! Driven through [`InstalledPackage`], the layer `QuiltSync` commands call,
//! with versioned physical keys as every real manifest carries them.

use std::path::Path;
use std::str::FromStr;

use super::*;

use aws_sdk_s3::primitives::ByteStream;
use test_log::test;

use crate::checksum::calculate_hash;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineage;
use crate::lineage::DomainLineageIo;
use crate::lineage::PackageLineageIo;
use crate::manifest::ManifestRow;
use crate::manifest::TopHasher;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

const BUCKET: &str = "b";

/// A published row for `body` at `logical_key`, pinned to S3 `version`.
async fn publish_row(
    remote: &MockRemote,
    scratch: &Path,
    logical_key: &str,
    version: &str,
    body: &[u8],
) -> Res<ManifestRow> {
    let storage = LocalStorage::new();
    let local = scratch.join(format!("{logical_key}.{version}"));
    storage
        .write_byte_stream(&local, ByteStream::from(body.to_vec()))
        .await?;
    let row = calculate_hash(
        &storage,
        &local,
        Path::new(logical_key),
        &HostConfig::default(),
    )
    .await?;
    let physical_key = format!("s3://{BUCKET}/f/a/{logical_key}?versionId={version}");
    remote
        .put_object(None, &S3Uri::from_str(&physical_key)?, body.to_vec())
        .await?;
    Ok(ManifestRow {
        physical_key,
        ..row
    })
}

/// Publishes a manifest under its real top hash, as a push would, and returns
/// that hash.
async fn publish_manifest(remote: &MockRemote, rows: Vec<ManifestRow>) -> Res<String> {
    let manifest = Manifest {
        rows,
        ..Manifest::default()
    };
    let mut hasher = TopHasher::new();
    hasher.append_header(&manifest.header)?;
    for row in &manifest.rows {
        hasher.append(row)?;
    }
    let hash = hasher.finalize();
    remote
        .put_object(
            None,
            &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/packages/{hash}"))?,
            ByteStream::from(&manifest),
        )
        .await?;
    Ok(hash)
}

/// Revision 1 installed, nothing downloaded yet. Latest is revision 2, which
/// modifies `changes.txt` and keeps `same.txt`.
struct Rev1Installed {
    package: InstalledPackage<LocalStorage, MockRemote>,
    rev1: String,
    rev2: String,
    _dirs: [tempfile::TempDir; 3],
}

impl Rev1Installed {
    async fn new() -> Res<Self> {
        let scratch = tempfile::TempDir::new()?;
        let remote = MockRemote::default();
        let namespace: Namespace = ("f", "a").into();

        let same = publish_row(&remote, scratch.path(), "same.txt", "s1", b"same").await?;
        let one = publish_row(&remote, scratch.path(), "changes.txt", "c1", b"one").await?;
        let two = publish_row(&remote, scratch.path(), "changes.txt", "c2", b"two").await?;
        let rev1 = publish_manifest(&remote, vec![one, same.clone()]).await?;
        let rev2 = publish_manifest(&remote, vec![two, same]).await?;
        remote
            .put_object(
                None,
                &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/named_packages/f/a/latest"))?,
                rev2.as_bytes().to_vec(),
            )
            .await?;

        let storage = LocalStorage::new();
        let (domain_lineage, home_dir) = DomainLineage::from_temp_dir()?;
        let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
        // Revision 1 explicitly — not latest.
        let domain_lineage = flow::install_package(
            domain_lineage,
            &paths,
            &storage,
            &remote,
            &ManifestUri {
                bucket: BUCKET.to_string(),
                namespace: namespace.clone(),
                hash: rev1.clone(),
                origin: None,
            },
        )
        .await?;
        let domain_io = DomainLineageIo::new(paths.lineage());
        domain_io.write(&storage, domain_lineage).await?;

        Ok(Self {
            package: InstalledPackage {
                lineage: PackageLineageIo::new(domain_io, namespace.clone()),
                paths,
                remote: Arc::new(remote),
                storage,
                namespace,
            },
            rev1,
            rev2,
            _dirs: [scratch, home_dir, paths_dir],
        })
    }

    fn all_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("changes.txt"), PathBuf::from("same.txt")]
    }

    /// The first half of [`InstalledPackage::pull`]: everything up to its
    /// lineage write, from the lineage as it reads it now.
    async fn pull_until_write(&self) -> Res<lineage::PackageLineage> {
        let p = &self.package;
        let (package_home, lineage) = p.lineage.read(&p.storage).await?;
        let mut manifest = p.manifest().await?;
        let (lineage, snapshot) = flow::snapshot_for_pull(
            lineage,
            &manifest,
            &p.paths,
            &p.storage,
            &*p.remote,
            &package_home,
            HostConfig::default(),
        )
        .await?;
        let (lineage, _) = flow::pull(
            lineage,
            &mut manifest,
            &p.paths,
            &p.storage,
            &*p.remote,
            package_home,
            snapshot,
            p.namespace.clone(),
            SyncScope::IndividualFiles,
        )
        .await?;
        Ok(lineage)
    }

    async fn changed(&self) -> Res<Vec<PathBuf>> {
        let status = self.package.status(Some(HostConfig::default())).await?;
        Ok(status.changes.keys().cloned().collect())
    }
}

/// The control: Select all + Download on revision 1 lands revision 1's bytes
/// and nothing reads as changed.
#[test(tokio::test)]
async fn downloading_an_older_revision_leaves_untouched_files_unchanged() -> Res {
    let t = Rev1Installed::new().await?;
    t.package.install_paths(&Rev1Installed::all_paths()).await?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.current_hash(), Some(t.rev1.as_str()));
    let home = t.package.package_home().await?;
    assert_eq!(tokio::fs::read(home.join("changes.txt")).await?, b"one");
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// Download, then an autopull tick pulls the package to latest: clean.
#[test(tokio::test)]
async fn a_pull_after_the_download_updates_the_file_cleanly() -> Res {
    let t = Rev1Installed::new().await?;
    t.package.install_paths(&Rev1Installed::all_paths()).await?;
    t.package
        .pull(Some(HostConfig::default()), SyncScope::IndividualFiles)
        .await?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.current_hash(), Some(t.rev2.as_str()));
    let home = t.package.package_home().await?;
    assert_eq!(tokio::fs::read(home.join("changes.txt")).await?, b"two");
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// `qhq-a4za`. The autopull tick pulls a `Behind` package, and an installed
/// older revision is `Behind` from the moment it is installed. Both
/// [`InstalledPackage::pull`] and [`InstalledPackage::install_paths`] read the
/// package lineage, await, and write the whole entry back, with no lock
/// between them. A pull that reads before the download writes, and writes
/// after it, re-points the package at revision 2 with the download's paths
/// dropped: the files on disk are revision 1's, now untracked, and the one
/// revision 2 modifies reads Modified against revision 2's row.
#[test(tokio::test)]
async fn a_pull_racing_the_download_leaves_no_file_reading_modified() -> Res {
    let t = Rev1Installed::new().await?;

    // The tick's pull starts first: its snapshot is the lineage as installed.
    let pulled = t.pull_until_write().await?;
    // Select all + Download completes while the pull is still in flight.
    t.package.install_paths(&Rev1Installed::all_paths()).await?;
    // The pull lands its lineage.
    t.package.lineage.write(&t.package.storage, pulled).await?;

    let changed = t.changed().await?;
    let lineage = t.package.lineage().await?;
    assert_eq!(
        changed,
        Vec::<PathBuf>::new(),
        "untouched files read as changed; now at {:?} ({}), tracking {:?}",
        lineage.current_hash(),
        if lineage.current_hash() == Some(t.rev2.as_str()) {
            "revision 2"
        } else {
            "revision 1"
        },
        lineage.paths.keys().collect::<Vec<_>>(),
    );
    Ok(())
}
