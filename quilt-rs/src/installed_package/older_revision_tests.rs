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
    dirs: [tempfile::TempDir; 3],
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
            dirs: [scratch, home_dir, paths_dir],
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

    /// A pull under the whole-package scope, which also fetches remote
    /// changes to paths this copy does not track.
    pub(super) async fn pull_entire(&self) -> Res {
        self.package
            .pull(Some(HostConfig::default()), SyncScope::EntirePackage)
            .await?;
        Ok(())
    }

    pub(super) async fn download(&self) -> Res {
        self.package.install_paths(&Self::all_paths()).await?;
        Ok(())
    }

    /// Publishes revision 3, which modifies `changes.txt` again and keeps
    /// `same.txt`, and moves `latest` to it. Returns its hash.
    pub(super) async fn publish_rev3(&self) -> Res<String> {
        let scratch = self.dirs[0].path();
        let same = publish_row(self.remote(), scratch, "same.txt", "s1", b"same").await?;
        let three = publish_row(self.remote(), scratch, "changes.txt", "c3", b"three").await?;
        let rev3 = publish_manifest(self.remote(), vec![three, same]).await?;
        self.remote()
            .put_object(
                None,
                &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/named_packages/f/a/latest"))?,
                rev3.as_bytes().to_vec(),
            )
            .await?;
        Ok(rev3)
    }

    /// A domain on this package's directory, sharing its storage and remote.
    pub(super) fn domain(&self) -> crate::LocalDomain<LocalStorage, MockRemote> {
        crate::LocalDomain::with_parts(
            self.package.paths.clone(),
            self.package.storage.clone(),
            Arc::clone(&self.package.remote),
        )
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
        self.remote().park(&format!(
            "s3://{BUCKET}/f/a/{logical_key}?versionId={version}"
        ))
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

/// Two packages written at once: an install of another package spans the
/// pull of this one. The install writes one entry, not the whole record it
/// read before its fetch, so the pull's entry survives.
#[test(tokio::test)]
async fn installing_another_package_during_a_pull_keeps_the_pull() -> Res {
    let t = Rev1Installed::new().await?;
    let other: Namespace = ("f", "b").into();
    t.remote()
        .put_object(
            None,
            &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/named_packages/f/b/latest"))?,
            t.rev1.as_bytes().to_vec(),
        )
        .await?;
    let domain = t.domain();
    let resolving_other = t
        .remote()
        .park(&format!("s3://{BUCKET}/.quilt/named_packages/f/b/latest"));

    let other_uri = ManifestUri {
        bucket: BUCKET.to_string(),
        namespace: other.clone(),
        hash: t.rev1.clone(),
        origin: None,
    };

    let (installed, pulled) = tokio::join!(domain.install_package(&other_uri), async {
        resolving_other.arrived().await;
        let pulled = t.pull().await;
        resolving_other.release();
        pulled
    },);
    pulled?;
    installed?;

    let lineage = domain.get_lineage().await?;
    assert_eq!(
        lineage.packages[&t.package.namespace].current_hash(),
        Some(t.rev2.as_str()),
        "the pull's entry survives the install"
    );
    assert_eq!(
        lineage.packages[&other].current_hash(),
        Some(t.rev1.as_str())
    );
    Ok(())
}

/// Waits for a parked fetch, failing rather than hanging when it never comes.
pub(super) async fn arrives(gate: &crate::io::remote::mocks::Gate, what: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), gate.arrived())
        .await
        .unwrap_or_else(|_| panic!("{what} never happened"));
}

fn is_changed_underneath(err: &Error, verb: &str) -> bool {
    matches!(
        err,
        Error::PackageOp(PackageOpError::ChangedUnderneath { verb: v, .. }) if *v == verb
    )
}

/// Two pulls at once: the second to write finds the entry already at latest,
/// which is its own result, and succeeds without writing.
#[test(tokio::test)]
async fn two_pulls_at_once_both_succeed() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (first, second) = tokio::join!(t.pull(), async {
        arrives(&fetching_rev2, "the first pull's fetch").await;
        let pulled = t.pull().await;
        fetching_rev2.release();
        pulled
    });
    first?;
    second?;

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

/// A commit lands while a pull works. The pull refuses and writes nothing:
/// re-pointing the package at latest would drop the commit. The file it
/// placed stays, as a local change against the commit.
#[test(tokio::test)]
async fn a_pull_crossed_by_a_commit_refuses_and_keeps_the_commit() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    let home = t.package.package_home().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (pulled, committed) = tokio::join!(t.pull(), async {
        arrives(&fetching_rev2, "the pull's fetch").await;
        tokio::fs::write(home.join("same.txt"), b"edited").await?;
        let committed = t
            .package
            .commit(
                "mine".to_string(),
                UserMeta::Keep,
                None,
                Some(HostConfig::default()),
            )
            .await;
        fetching_rev2.release();
        committed
    });
    let commit = committed?;
    let err = pulled.expect_err("the pull must refuse");
    assert!(is_changed_underneath(&err, "pulling"), "{err}");
    assert_eq!(err.to_string(), "f/a changed while pulling; try again");

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.commit.map(|c| c.hash), Some(commit.hash));
    assert_eq!(lineage.base_hash, t.rev1);
    Ok(())
}

/// A pull crossed by an uninstall of a path it tracked: the pull writes
/// second and leaves the path out, rather than tracking a file that is gone.
#[test(tokio::test)]
async fn a_pull_crossed_by_an_uninstall_leaves_the_path_out() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    let home = t.package.package_home().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (pulled, uninstalled) = tokio::join!(t.pull(), async {
        arrives(&fetching_rev2, "the pull's fetch").await;
        let uninstalled = t
            .package
            .uninstall_paths(&vec![PathBuf::from("same.txt")])
            .await;
        fetching_rev2.release();
        uninstalled
    });
    uninstalled?;
    pulled?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.current_hash(), Some(t.rev2.as_str()));
    assert_eq!(
        lineage.paths.keys().cloned().collect::<Vec<_>>(),
        vec![PathBuf::from("changes.txt")]
    );
    assert!(!home.join("same.txt").exists());
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// The other order: the uninstall read the package before a pull landed and
/// writes after it. The uninstall wins, and the pull's move to revision 2
/// survives: the uninstall removes its one path, not the pull's revision.
#[test(tokio::test)]
async fn an_uninstall_crossed_by_a_pull_wins_and_keeps_the_pull() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    let home = t.package.package_home().await?;
    let changes = home.join("changes.txt");
    let parked = ParkedStorage::new(&changes);
    let parked_package = parked.package(&t);

    let to_remove = vec![PathBuf::from("changes.txt")];
    let (uninstalled, pulled) = tokio::join!(parked_package.uninstall_paths(&to_remove), async {
        arrives(&parked.gate, "the uninstall's removal").await;
        let pulled = t.pull().await;
        parked.gate.release();
        pulled
    },);
    pulled?;
    uninstalled?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.current_hash(), Some(t.rev2.as_str()));
    assert_eq!(
        lineage.paths.keys().cloned().collect::<Vec<_>>(),
        vec![PathBuf::from("same.txt")]
    );
    assert!(!changes.exists());
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// A download that finishes after its package was uninstalled writes nothing,
/// says the package is not installed, and removes the files it placed.
#[test(tokio::test)]
async fn a_download_after_the_package_was_uninstalled_does_not_bring_it_back() -> Res {
    let t = Rev1Installed::new().await?;
    let home = t.package.package_home().await?;
    let domain = t.domain();
    let fetching_changes = t.park_object("changes.txt", "c1");

    let (downloaded, uninstalled) = tokio::join!(t.download(), async {
        arrives(&fetching_changes, "the download's fetch").await;
        let uninstalled = domain.uninstall_package(t.package.namespace.clone()).await;
        fetching_changes.release();
        uninstalled
    });
    uninstalled?;
    let err = downloaded.expect_err("the download must not re-create the package");
    assert!(
        matches!(
            err,
            Error::InstallPackage(crate::InstallPackageError::NotInstalled(_))
        ),
        "{err}"
    );

    assert!(
        !domain
            .get_lineage()
            .await?
            .packages
            .contains_key(&t.package.namespace)
    );
    assert!(!home.join("changes.txt").exists());
    assert!(!home.join("same.txt").exists());
    Ok(())
}

/// One redo round, then refuse. The pull's write finds a download landed and
/// reconciles it; while it does, another download lands, and the pull gives up
/// rather than redo again.
#[test(tokio::test)]
async fn a_pull_that_keeps_being_crossed_refuses_after_one_redo() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);
    let redoing = t.park_object("changes.txt", "c2");

    let (pulled, downloads) = tokio::join!(t.pull(), async {
        arrives(&fetching_rev2, "the pull's fetch").await;
        t.package
            .install_paths(&[PathBuf::from("changes.txt")])
            .await?;
        fetching_rev2.release();
        arrives(&redoing, "the pull's redo").await;
        t.package
            .install_paths(&[PathBuf::from("same.txt")])
            .await?;
        redoing.release();
        Res::Ok(())
    });
    downloads?;
    let err = pulled.expect_err("the pull must refuse");
    assert!(is_changed_underneath(&err, "pulling"), "{err}");

    // The refusal leaves the downloads as they landed: its redo had put
    // revision 2's bytes over `changes.txt`, and it puts the download's back.
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

/// A download whose redo is crossed again refuses, and removes the files it
/// placed, so pressing Download again works.
#[test(tokio::test)]
async fn a_download_that_keeps_being_crossed_refuses_and_can_be_retried() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_changes = t.park_object("changes.txt", "c1");
    let redoing = t.park_object("changes.txt", "c2");

    let (downloaded, pulls) = tokio::join!(t.download(), async {
        arrives(&fetching_changes, "the download's fetch").await;
        let first = t.pull().await;
        fetching_changes.release();
        first?;
        arrives(&redoing, "the download's redo").await;
        // A reset, not a pull: the download's files are on disk untracked,
        // and a pull would refuse over them.
        let second = async {
            t.publish_rev3().await?;
            t.package.reset_to_latest().await
        }
        .await;
        redoing.release();
        second?;
        Res::Ok(())
    });
    pulls?;
    let err = downloaded.expect_err("the download must refuse");
    assert!(is_changed_underneath(&err, "downloading"), "{err}");
    let home = t.package.package_home().await?;
    assert!(!home.join("changes.txt").exists());
    assert!(!home.join("same.txt").exists());

    t.download().await?;
    let rev3 = t
        .package
        .lineage()
        .await?
        .current_hash()
        .map(str::to_string);
    assert_eq!(
        t.outcome().await?,
        (rev3, Rev1Installed::all_paths(), b"three".to_vec())
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// A pull's redo that finds a crossed file edited since the download, where
/// latest changes it too, refuses like a pull that finds any conflict: it
/// writes nothing and overwrites nothing. The next tick sees it as blocked.
#[test(tokio::test)]
async fn a_pull_whose_redo_finds_an_edited_download_refuses() -> Res {
    let t = Rev1Installed::new().await?;
    let home = t.package.package_home().await?;
    // Parked once its work is done: an edit that landed before its walk
    // would be an ordinary pull conflict, not the redo's.
    let parked = ParkedStorage::at_the_write_step();
    let parked_puller = parked.package(&t);

    let (pulled, downloaded) = tokio::join!(
        parked_puller.pull(Some(HostConfig::default()), SyncScope::IndividualFiles),
        async {
            arrives(&parked.gate, "the pull's write step").await;
            let edited = async {
                t.download().await?;
                tokio::fs::write(home.join("changes.txt"), b"edited").await?;
                Res::Ok(())
            }
            .await;
            parked.gate.release();
            edited
        }
    );
    downloaded?;
    let err = pulled.expect_err("the pull must refuse");
    assert!(is_changed_underneath(&err, "pulling"), "{err}");

    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev1.clone()),
            Rev1Installed::all_paths(),
            b"edited".to_vec()
        )
    );
    assert_eq!(t.changed().await?, vec![PathBuf::from("changes.txt")]);
    Ok(())
}

/// An uninstall and a download both land while a pull works. The pull's redo
/// for the download must still leave the uninstalled path out.
#[test(tokio::test)]
async fn a_pull_crossed_by_an_uninstall_and_a_download_keeps_both() -> Res {
    let t = Rev1Installed::new().await?;
    t.package
        .install_paths(&[PathBuf::from("same.txt")])
        .await?;
    let home = t.package.package_home().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (pulled, others) = tokio::join!(t.pull(), async {
        arrives(&fetching_rev2, "the pull's fetch").await;
        t.package
            .uninstall_paths(&vec![PathBuf::from("same.txt")])
            .await?;
        t.package
            .install_paths(&[PathBuf::from("changes.txt")])
            .await?;
        fetching_rev2.release();
        Res::Ok(())
    });
    others?;
    pulled?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.current_hash(), Some(t.rev2.as_str()));
    assert_eq!(
        lineage.paths.keys().cloned().collect::<Vec<_>>(),
        vec![PathBuf::from("changes.txt")]
    );
    assert_eq!(tokio::fs::read(home.join("changes.txt")).await?, b"two");
    assert!(!home.join("same.txt").exists());
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// [`LocalStorage`], except that one operation parks until the gate is
/// released, once: removing one file (the only way to land another writer
/// inside `uninstall_paths`, which never touches the remote), or taking the
/// lineage lock (the moment a writer's work is done and its write step starts).
#[derive(Clone)]
pub(super) struct ParkedStorage {
    inner: LocalStorage,
    park: Park,
    armed: Arc<std::sync::atomic::AtomicBool>,
    pub(super) gate: Arc<crate::io::remote::mocks::Gate>,
}

#[derive(Clone)]
enum Park {
    Remove(PathBuf),
    Lock,
}

impl ParkedStorage {
    fn new(path: &Path) -> Self {
        Self::parking(Park::Remove(path.to_path_buf()))
    }

    pub(super) fn at_the_write_step() -> Self {
        Self::parking(Park::Lock)
    }

    fn parking(park: Park) -> Self {
        Self {
            inner: LocalStorage::new(),
            park,
            armed: Arc::new(true.into()),
            gate: Arc::default(),
        }
    }

    async fn maybe_park(&self, now: &Park, path: &Path) {
        let hit = match (&self.park, now) {
            (Park::Remove(target), Park::Remove(_)) => target == path,
            (Park::Lock, Park::Lock) => true,
            _ => false,
        };
        if hit && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.gate.hold().await;
        }
    }

    /// A second handle on `t`'s package that goes through this storage.
    fn package(&self, t: &Rev1Installed) -> InstalledPackage<ParkedStorage, MockRemote> {
        self.handle(&t.package)
    }

    /// A second handle on `package` that goes through this storage.
    pub(super) fn handle(
        &self,
        package: &InstalledPackage<LocalStorage, MockRemote>,
    ) -> InstalledPackage<ParkedStorage, MockRemote> {
        InstalledPackage {
            lineage: package.lineage.clone(),
            paths: package.paths.clone(),
            remote: Arc::clone(&package.remote),
            storage: self.clone(),
            namespace: package.namespace.clone(),
        }
    }
}

impl Storage for ParkedStorage {
    async fn copy(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res<u64> {
        self.inner.copy(from, to).await
    }

    async fn create_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
        self.inner.create_dir_all(path).await
    }

    async fn create_file(&self, path: impl AsRef<Path>) -> Res<tokio::fs::File> {
        self.inner.create_file(path).await
    }

    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        self.inner.exists(path).await
    }

    async fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> Res<chrono::DateTime<chrono::Utc>> {
        self.inner.modified_timestamp(path).await
    }

    async fn open_file(&self, path: impl AsRef<Path> + Send) -> Res<tokio::fs::File> {
        self.inner.open_file(path).await
    }

    async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
        self.inner.read_byte_stream(path).await
    }

    async fn read_dir(&self, path: impl AsRef<Path> + Send + Sync) -> Res<tokio::fs::ReadDir> {
        self.inner.read_dir(path).await
    }

    async fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
        self.inner.remove_dir_all(path).await
    }

    async fn remove_file(&self, path: impl AsRef<Path> + Send) -> std::io::Result<()> {
        self.maybe_park(&Park::Remove(PathBuf::new()), path.as_ref())
            .await;
        self.inner.remove_file(path).await
    }

    async fn rename(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res {
        self.inner.rename(from, to).await
    }

    async fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> Res {
        self.inner.write_byte_stream(path, body).await
    }

    async fn lock_exclusive(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> Res<crate::io::storage::LockGuard> {
        self.maybe_park(&Park::Lock, path.as_ref()).await;
        self.inner.lock_exclusive(path).await
    }
}

/// Another pull got to latest first, and then an uninstall removed a path this
/// pull placed. This pull writes nothing, as the entry is already where it was
/// going, but the file it placed must not stay behind untracked.
#[test(tokio::test)]
async fn a_pull_beaten_to_latest_removes_a_file_uninstalled_meanwhile() -> Res {
    let t = Rev1Installed::new().await?;
    let home = t.package.package_home().await?;
    // Whole-package scope: the untracked `changes.txt` is fetched, and placed
    // with no check of what is there.
    let fetching = t.park_object("changes.txt", "c2");

    let (first, others) = tokio::join!(t.pull_entire(), async {
        arrives(&fetching, "the first pull's fetch").await;
        let others = async {
            t.pull_entire().await?;
            t.package
                .uninstall_paths(&vec![PathBuf::from("changes.txt")])
                .await?;
            Res::Ok(())
        }
        .await;
        fetching.release();
        others
    });
    others?;
    first?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.current_hash(), Some(t.rev2.as_str()));
    assert!(lineage.paths.is_empty());
    assert!(
        !home.join("changes.txt").exists(),
        "the uninstalled file must not come back"
    );
    Ok(())
}
