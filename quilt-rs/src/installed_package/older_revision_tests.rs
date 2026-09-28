//! Install an older revision, download its files, and let a pull of the
//! package start while the download runs: no file nobody touched may read
//! Modified.
//!
//! Driven through [`InstalledPackage`], the layer `QuiltSync` commands call,
//! with versioned physical keys as every real manifest carries them. The mock
//! remote parks one fetch, so the other operation starts inside it: the first
//! writer has read the package's lineage and not yet written it.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use super::*;

use aws_sdk_s3::primitives::ByteStream;
use test_log::test;

use crate::checksum::calculate_hash;
use crate::io::remote::mocks::Gate;
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

    /// Revision 2 with both files downloaded, and nothing reading changed.
    pub(super) async fn assert_clean_at_rev2(&self) -> Res {
        assert_eq!(
            self.outcome().await?,
            (
                Some(self.rev2.clone()),
                Self::all_paths(),
                b"two".to_vec()
            )
        );
        assert_eq!(self.changed().await?, Vec::<PathBuf>::new());
        Ok(())
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
    pub(super) fn park_manifest(&self, hash: &str) -> Arc<Gate> {
        self.remote()
            .park(&format!("s3://{BUCKET}/.quilt/packages/{hash}"))
    }
}

/// Waits for a parked fetch, failing rather than hanging when it never comes.
pub(super) async fn arrives(gate: &Gate, what: &str) {
    tokio::time::timeout(Duration::from_secs(10), gate.arrived())
        .await
        .unwrap_or_else(|_| panic!("{what} never happened"));
}

/// Runs `second` while `gate`'s fetch is parked in the first writer, gives it
/// `patience` to finish there, then lets the first writer go and waits for
/// `second` to end. Says whether `second` finished while the first was
/// parked.
pub(super) async fn inside<T>(
    gate: &Gate,
    patience: Duration,
    second: impl Future<Output = T>,
) -> (T, bool) {
    arrives(gate, "the first writer's fetch").await;
    let mut second = std::pin::pin!(second);
    let early = tokio::time::timeout(patience, &mut second).await;
    gate.release();
    match early {
        Ok(out) => (out, true),
        Err(_) => (second.await, false),
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
    t.assert_clean_at_rev2().await
}

/// The autopull tick pulls a `Behind` package, and an installed older revision
/// is `Behind` from the moment it is installed. The pull reads the lineage,
/// and a download starts while the pull fetches revision 2. If the download
/// wrote before the pull, the pull's write would re-point the package at
/// revision 2 with the downloaded paths dropped, and `changes.txt` would read
/// Modified. The download waits out the pull instead, then downloads
/// revision 2's bytes.
#[test(tokio::test)]
async fn a_pull_racing_the_download_leaves_no_file_reading_modified() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (pulled, (downloaded, _)) = tokio::join!(
        t.pull(),
        inside(&fetching_rev2, Duration::from_millis(300), t.download())
    );
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
    t.assert_clean_at_rev2().await
}
