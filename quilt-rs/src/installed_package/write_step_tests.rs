//! Each remaining writer's rule when another writer lands inside it: the one
//! that writes second applies only its own change, redoes it, or refuses, and
//! never drops what the other recorded.
//!
//! [`super::older_revision_tests`] pins the pull, the download and the
//! uninstall; these pin the rest.

use std::str::FromStr;

use super::older_revision_tests::ParkedStorage;
use super::older_revision_tests::Rev1Installed;
use super::older_revision_tests::arrives;
use super::*;

use aws_sdk_s3::primitives::ByteStream;
use test_log::test;

use crate::io::remote::mocks::MockRemote;
use crate::paths::DomainPaths;
use quilt_uri::S3Uri;

fn is_changed_underneath(err: &Error, verb: &str) -> bool {
    matches!(
        err,
        Error::PackageOp(PackageOpError::ChangedUnderneath { verb: v, .. }) if *v == verb
    )
}

async fn commit(package: &InstalledPackage<LocalStorage, MockRemote>) -> Res<CommitState> {
    package
        .commit(
            "a change".to_string(),
            UserMeta::Keep,
            None,
            Some(HostConfig::default()),
        )
        .await
}

/// A commit that changes only the package's metadata.
async fn commit_meta(
    package: &InstalledPackage<LocalStorage, MockRemote>,
    version: u32,
) -> Res<CommitState> {
    package
        .commit(
            format!("meta {version}"),
            UserMeta::Set(serde_json::json!({ "version": version })),
            None,
            Some(HostConfig::default()),
        )
        .await
}

/// A package made locally, with no remote: `a.txt` committed once.
struct LocalPackage {
    package: InstalledPackage<LocalStorage, MockRemote>,
    home: PathBuf,
    _dirs: [tempfile::TempDir; 3],
}

impl LocalPackage {
    async fn new() -> Res<Self> {
        let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
        let home_dir = tempfile::TempDir::new()?;
        let source = tempfile::TempDir::new()?;
        tokio::fs::write(source.path().join("a.txt"), b"a").await?;
        let domain = crate::LocalDomain::with_parts(
            paths,
            LocalStorage::new(),
            Arc::new(MockRemote::default()),
        );
        domain.set_home(home_dir.path()).await?;
        let namespace: Namespace = ("me", "local").into();
        let package = domain
            .create_package(
                namespace,
                Some(source.path().to_path_buf()),
                Some("first".to_string()),
            )
            .await?;
        let home = package.package_home().await?;
        Ok(Self {
            package,
            home,
            _dirs: [paths_dir, home_dir, source],
        })
    }

    async fn edit_and_commit(&self, body: &[u8]) -> Res<CommitState> {
        tokio::fs::write(self.home.join("a.txt"), body).await?;
        commit(&self.package).await
    }
}

/// A download lands while a commit hashes the tree. The commit redoes itself
/// from the entry it found, so the downloaded path stays tracked.
#[test(tokio::test)]
async fn a_commit_crossed_by_a_download_redoes_and_keeps_the_download() -> Res {
    let t = Rev1Installed::new().await?;
    t.package
        .install_paths(&[PathBuf::from("changes.txt")])
        .await?;
    let home = t.package.package_home().await?;
    tokio::fs::write(home.join("changes.txt"), b"mine").await?;
    let parked = ParkedStorage::at_the_write_step();
    let parked_handle = parked.handle(&t.package);

    let (committed, downloaded) = tokio::join!(
        parked_handle.commit(
            "mine".to_string(),
            UserMeta::Keep,
            None,
            Some(HostConfig::default()),
        ),
        async {
            arrives(&parked.gate, "the commit's write step").await;
            let downloaded = t.package.install_paths(&[PathBuf::from("same.txt")]).await;
            parked.gate.release();
            downloaded
        }
    );
    downloaded?;
    let commit = committed?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.commit.map(|c| c.hash), Some(commit.hash));
    assert_eq!(
        lineage.paths.keys().cloned().collect::<Vec<_>>(),
        Rev1Installed::all_paths()
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

/// A commit lands on top of the one a push is uploading. The push's remote
/// fields land, and the newer commit stays pending.
#[test(tokio::test)]
async fn a_push_crossed_by_a_newer_commit_keeps_it_pending() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    // Metadata commits: the mock's upload cannot reproduce a local file's
    // pushed row, and the push only has to upload something.
    commit_meta(&t.package, 1).await?;
    // The push reads the tag after its upload, to learn whether it may
    // certify: by then the revision is on the remote.
    let uploaded = t.remote().park("s3://b/.quilt/named_packages/f/a/latest");

    let (pushed, newer) = tokio::join!(t.package.push(Some(HostConfig::default())), async {
        arrives(&uploaded, "the push's tag read").await;
        let newer = commit_meta(&t.package, 2).await;
        uploaded.release();
        newer
    });
    let newer = newer?;
    let pushed = pushed?;

    let lineage = t.package.lineage().await?;
    assert_eq!(
        lineage.remote_uri.as_ref().map(|r| r.hash.clone()),
        Some(pushed.manifest_uri.hash.clone()),
        "the pushed hash is a fact"
    );
    assert_eq!(
        lineage.commit.map(|c| c.hash),
        Some(newer.hash),
        "the newer commit stays pending"
    );
    Ok(())
}

/// A commit lands while certify tags the remote hash as latest. The tag is
/// a fact and lands; the commit stays.
#[test(tokio::test)]
async fn a_certify_crossed_by_a_commit_keeps_the_commit() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    let home = t.package.package_home().await?;
    let tagging = t.remote().park("s3://b/.quilt/named_packages/f/a/latest");

    let (certified, committed) = tokio::join!(t.package.certify_latest(), async {
        arrives(&tagging, "certify's tag write").await;
        let committed = async {
            tokio::fs::write(home.join("same.txt"), b"mine").await?;
            commit(&t.package).await
        }
        .await;
        tagging.release();
        committed
    });
    let committed = committed?;
    certified?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.latest_hash, t.rev1, "the tag landed");
    assert_eq!(lineage.base_hash, t.rev1);
    assert_eq!(lineage.commit.map(|c| c.hash), Some(committed.hash));
    Ok(())
}

/// A download lands while a reset works. The reset redoes as a pull would:
/// the downloaded files end at latest, clean.
#[test(tokio::test)]
async fn a_reset_crossed_by_a_download_updates_the_downloaded_files() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (reset, downloaded) = tokio::join!(t.package.reset_to_latest(), async {
        arrives(&fetching_rev2, "the reset's fetch").await;
        let downloaded = t.download().await;
        fetching_rev2.release();
        downloaded
    });
    downloaded?;
    reset?;

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

/// The scope is only ever written by `set_sync_scope`, so a pull that writes
/// after it keeps it.
#[test(tokio::test)]
async fn a_scope_set_during_a_pull_survives_it() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_rev2 = t.park_manifest(&t.rev2);

    let (pulled, scoped) = tokio::join!(t.pull(), async {
        arrives(&fetching_rev2, "the pull's fetch").await;
        let scoped = t.package.set_sync_scope(SyncScope::EntirePackage).await;
        fetching_rev2.release();
        scoped
    });
    scoped?;
    pulled?;

    let lineage = t.package.lineage().await?;
    assert_eq!(lineage.sync_scope, SyncScope::EntirePackage);
    assert_eq!(lineage.current_hash(), Some(t.rev2.as_str()));
    Ok(())
}

/// A commit lands while undo restores the previous revision: undo refuses,
/// writes nothing, and the new commit stands.
#[test(tokio::test)]
async fn an_undo_crossed_by_a_commit_refuses() -> Res {
    let p = LocalPackage::new().await?;
    p.edit_and_commit(b"a2").await?;
    let parked = ParkedStorage::at_the_write_step();
    let undoer = parked.handle(&p.package);

    let (undone, committed) = tokio::join!(undoer.undo_commit(), async {
        arrives(&parked.gate, "the undo's write step").await;
        let committed = p.edit_and_commit(b"a3").await;
        parked.gate.release();
        committed
    });
    let committed = committed?;
    let err = undone.expect_err("the undo must refuse");
    assert!(is_changed_underneath(&err, "undoing the commit"), "{err}");
    assert_eq!(
        p.package.lineage().await?.commit.map(|c| c.hash),
        Some(committed.hash)
    );
    Ok(())
}

/// A commit lands while set remote checks the bucket: set remote refuses and
/// writes nothing, and the commit stands.
#[test(tokio::test)]
async fn a_set_remote_crossed_by_a_commit_refuses() -> Res {
    let p = LocalPackage::new().await?;
    let parked = ParkedStorage::at_the_write_step();
    let setter = parked.handle(&p.package);

    let (set, committed) = tokio::join!(
        setter.set_remote(
            "bucket".to_string(),
            None,
            crate::io::remote::WorkflowIntent::BucketDefault,
        ),
        async {
            arrives(&parked.gate, "set remote's write step").await;
            let committed = p.edit_and_commit(b"a2").await;
            parked.gate.release();
            committed
        }
    );
    let committed = committed?;
    let err = set.expect_err("set remote must refuse");
    assert!(is_changed_underneath(&err, "setting the remote"), "{err}");
    let lineage = p.package.lineage().await?;
    assert_eq!(lineage.remote_uri, None);
    assert_eq!(lineage.commit.map(|c| c.hash), Some(committed.hash));
    Ok(())
}

/// Two installs of one package cross: the second to write finds it already
/// installed and refuses, leaving the first's entry.
#[test(tokio::test)]
async fn two_installs_of_one_package_cross_and_the_second_refuses() -> Res {
    let t = Rev1Installed::new().await?;
    let domain = t.domain();
    let latest_of_b = "s3://b/.quilt/named_packages/f/b/latest";
    t.remote()
        .put_object(
            None,
            &S3Uri::from_str(latest_of_b)?,
            ByteStream::from(t.rev1.as_bytes().to_vec()),
        )
        .await?;
    let uri = ManifestUri {
        bucket: "b".to_string(),
        namespace: ("f", "b").into(),
        hash: t.rev1.clone(),
        origin: None,
    };
    let resolving = t.remote().park(latest_of_b);

    let (first, second) = tokio::join!(domain.install_package(&uri), async {
        arrives(&resolving, "the first install's tag read").await;
        let second = domain.install_package(&uri).await;
        resolving.release();
        second
    });
    second?;
    let err = first.err().expect("the first install must refuse");
    assert!(
        matches!(
            err,
            Error::InstallPackage(crate::InstallPackageError::AlreadyInstalled(_))
        ),
        "{err}"
    );
    let lineage = domain.get_lineage().await?;
    assert!(lineage.packages.contains_key(&uri.namespace));
    assert!(
        lineage.packages.contains_key(&t.package.namespace),
        "the other package's entry is untouched"
    );
    Ok(())
}
