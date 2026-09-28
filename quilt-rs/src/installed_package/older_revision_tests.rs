//! Install an older revision, download its files, and let a pull of the
//! package cross the download: no file nobody touched may read Modified.
//!
//! Driven through [`InstalledPackage`], the layer `QuiltSync` commands call,
//! with versioned physical keys as every real manifest carries them. The mock
//! remote parks one fetch, so the other operation lands inside it: the first
//! writer has read the package's lineage and not yet written it.

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
pub(super) struct Rev1Installed {
    pub(super) package: InstalledPackage<LocalStorage, MockRemote>,
    pub(super) rev1: String,
    pub(super) rev2: String,
    _dirs: [tempfile::TempDir; 3],
}

impl Rev1Installed {
    pub(super) async fn new() -> Res<Self> {
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

    pub(super) fn all_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("changes.txt"), PathBuf::from("same.txt")]
    }

    pub(super) async fn changed(&self) -> Res<Vec<PathBuf>> {
        let status = self.package.status(Some(HostConfig::default())).await?;
        Ok(status.changes.keys().cloned().collect())
    }

    /// Where the package is, what its lineage tracks, and `changes.txt`'s bytes.
    pub(super) async fn outcome(&self) -> Res<(Option<String>, Vec<PathBuf>, Vec<u8>)> {
        let lineage = self.package.lineage().await?;
        let home = self.package.package_home().await?;
        Ok((
            lineage.current_hash().map(str::to_string),
            lineage.paths.keys().cloned().collect(),
            tokio::fs::read(home.join("changes.txt")).await?,
        ))
    }

    pub(super) async fn pull(&self) -> Res {
        self.package
            .pull(Some(HostConfig::default()), SyncScope::IndividualFiles)
            .await?;
        Ok(())
    }

    pub(super) async fn download(&self) -> Res {
        self.package.install_paths(&Self::all_paths()).await?;
        Ok(())
    }

    pub(super) fn remote(&self) -> &MockRemote {
        &self.package.remote
    }

    /// Parks the next fetch of the manifest `hash`.
    pub(super) fn park_manifest(&self, hash: &str) -> Arc<crate::io::remote::mocks::Gate> {
        self.remote()
            .park(&format!("s3://{BUCKET}/.quilt/packages/{hash}"))
    }

    /// Parks the next fetch of `logical_key` at S3 `version`.
    pub(super) fn park_object(
        &self,
        logical_key: &str,
        version: &str,
    ) -> Arc<crate::io::remote::mocks::Gate> {
        self.remote()
            .park(&format!("s3://{BUCKET}/f/a/{logical_key}?versionId={version}"))
    }
}

/// The control: Select all + Download on revision 1 lands revision 1's bytes
/// and nothing reads as changed.
#[test(tokio::test)]
async fn downloading_an_older_revision_leaves_untouched_files_unchanged() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;

    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev1.clone()),
            Rev1Installed::all_paths(),
            b"one".to_vec()
        )
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// Download, then an autopull tick pulls the package to latest: clean.
#[test(tokio::test)]
async fn a_pull_after_the_download_updates_the_file_cleanly() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    t.pull().await?;

    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev2.clone()),
            Rev1Installed::all_paths(),
            b"two".to_vec()
        )
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// The autopull tick pulls a `Behind` package, and an installed older revision
/// is `Behind` from the moment it is installed. The pull reads the lineage,
/// the download lands while the pull fetches revision 2, and then the pull
/// writes. Its write must not re-point the package at revision 2 with the
/// downloaded paths dropped, or with them left at revision 1's rows: either
/// way `changes.txt` reads Modified.
#[test(tokio::test)]
async fn a_pull_racing_the_download_leaves_no_file_reading_modified() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (pulled, downloaded) = tokio::join!(t.pull(), async {
        fetching_rev2.arrived().await;
        let downloaded = t.download().await;
        fetching_rev2.release();
        downloaded
    });
    downloaded?;
    pulled?;

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
    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev2.clone()),
            Rev1Installed::all_paths(),
            b"two".to_vec()
        )
    );
    Ok(())
}

/// The reverse order: the download reads the lineage, the pull lands while the
/// download fetches its first file, and then the download writes. It finds
/// the package at revision 2 and installs the same paths again from there.
#[test(tokio::test)]
async fn a_download_racing_the_pull_installs_from_the_new_revision() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_changes = t.park_object("changes.txt", "c1");

    let (downloaded, pulled) = tokio::join!(t.download(), async {
        fetching_changes.arrived().await;
        let pulled = t.pull().await;
        fetching_changes.release();
        pulled
    });
    pulled?;
    downloaded?;

    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev2.clone()),
            Rev1Installed::all_paths(),
            b"two".to_vec()
        )
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}
