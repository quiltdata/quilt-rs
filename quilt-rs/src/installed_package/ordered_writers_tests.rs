//! A pull and a download of the same package, ordered by one lock per package
//! as `QuiltSync` orders them: the finished package tracks what was downloaded,
//! and no file nobody touched reads Modified.
//!
//! Unordered, the two race. Each reads the package's lineage, awaits, and
//! writes the whole entry back, so a pull that writes last re-points the
//! package at the newer revision with the downloaded paths gone. These tests
//! pin the outcome the ordering buys, on a real [`InstalledPackage`] with
//! versioned physical keys. quilt-rs itself holds no lock yet, so the tests
//! supply one.

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

    async fn changed(&self) -> Res<Vec<PathBuf>> {
        let status = self.package.status(Some(HostConfig::default())).await?;
        Ok(status.changes.keys().cloned().collect())
    }

    /// Where the package is, what its lineage tracks, and `changes.txt`'s bytes.
    async fn outcome(&self) -> Res<(Option<String>, Vec<PathBuf>, Vec<u8>)> {
        let lineage = self.package.lineage().await?;
        let home = self.package.package_home().await?;
        Ok((
            lineage.current_hash().map(str::to_string),
            lineage.paths.keys().cloned().collect(),
            tokio::fs::read(home.join("changes.txt")).await?,
        ))
    }

    async fn pull(&self) -> Res {
        self.package
            .pull(Some(HostConfig::default()), SyncScope::IndividualFiles)
            .await?;
        Ok(())
    }

    async fn download(&self) -> Res {
        self.package.install_paths(&Self::all_paths()).await?;
        Ok(())
    }
}

/// A download that starts while a pull holds the package waits it out, then
/// installs against the revision the pull reached.
#[test(tokio::test)]
async fn a_download_that_waits_out_a_pull_lands_on_the_new_revision_unmodified() -> Res {
    let t = Rev1Installed::new().await?;
    let lock = tokio::sync::Mutex::new(());
    // `join!` polls the pull first, and an uncontended lock is taken on that
    // first poll, so the download queues behind it.
    let (pulled, downloaded) = tokio::join!(
        async {
            let _ordered = lock.lock().await;
            t.pull().await
        },
        async {
            let _ordered = lock.lock().await;
            t.download().await
        },
    );
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

/// A hand-pressed pull that starts during a download waits it out, then
/// updates the downloaded files.
#[test(tokio::test)]
async fn a_pull_that_waits_out_a_download_updates_the_downloaded_files_unmodified() -> Res {
    let t = Rev1Installed::new().await?;
    let lock = tokio::sync::Mutex::new(());
    let (downloaded, pulled) = tokio::join!(
        async {
            let _ordered = lock.lock().await;
            t.download().await
        },
        async {
            let _ordered = lock.lock().await;
            t.pull().await
        },
    );
    downloaded?;
    pulled?;

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

/// The tick's side: it only tries the lock. While a download holds it the tick
/// skips, the download stands on revision 1, and the next tick pulls cleanly.
#[test(tokio::test)]
async fn a_tick_that_skips_during_a_download_leaves_it_intact_and_pulls_next_time() -> Res {
    let t = Rev1Installed::new().await?;
    let lock = tokio::sync::Mutex::new(());
    {
        let _downloading = lock.lock().await;
        assert!(lock.try_lock().is_err(), "the tick must see the download");
        t.download().await?;
    }
    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev1.clone()),
            Rev1Installed::all_paths(),
            b"one".to_vec()
        )
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());

    let _ordered = lock
        .try_lock()
        .expect("the next tick finds the package free");
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
