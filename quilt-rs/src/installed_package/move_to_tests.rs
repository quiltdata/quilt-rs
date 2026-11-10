//! A Diverged copy whose own change `latest` already holds, moved onto
//! `latest`: what the other side changed is written, and the folder's own
//! edits are kept or refuse the move.

use std::path::Path;

use super::*;

use test_log::test;

use super::older_revision_tests::BUCKET;
use super::older_revision_tests::arrives;
use super::older_revision_tests::publish_manifest;
use super::older_revision_tests::publish_row;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineage;
use crate::lineage::DomainLineageIo;
use crate::lineage::PackageLineageIo;
use crate::paths::DomainPaths;

/// The copy changed `a.txt` and pushed it as `ours`; the other side changed
/// `b.txt`; `merged` holds both and is `latest`. The copy is installed at
/// `ours` with both files downloaded, its base still the revision both
/// sides started from.
struct Diverged {
    package: InstalledPackage<LocalStorage, MockRemote>,
    base: String,
    ours: String,
    merged: String,
    _dirs: [tempfile::TempDir; 3],
}

impl Diverged {
    async fn new() -> Res<Self> {
        let scratch = tempfile::TempDir::new()?;
        let remote = MockRemote::default();
        let namespace: Namespace = ("f", "a").into();
        let row = |key, version, body: &'static [u8]| {
            publish_row(&remote, scratch.path(), key, version, body)
        };
        let (a1, a_mine) = (
            row("a.txt", "a1", b"1").await?,
            row("a.txt", "a2", b"mine").await?,
        );
        let (b1, b_theirs) = (
            row("b.txt", "b1", b"1").await?,
            row("b.txt", "b2", b"theirs").await?,
        );
        let base = publish_manifest(&remote, vec![a1, b1.clone()]).await?;
        let ours = publish_manifest(&remote, vec![a_mine.clone(), b1]).await?;
        let merged = publish_manifest(&remote, vec![a_mine, b_theirs]).await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from(
                    format!("s3://{BUCKET}/.quilt/named_packages/f/a/latest").as_str(),
                )?,
                merged.as_bytes().to_vec(),
            )
            .await?;

        let storage = LocalStorage::new();
        let (domain_lineage, home_dir) = DomainLineage::from_temp_dir()?;
        let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
        let domain_lineage = flow::install_package(
            domain_lineage,
            &paths,
            &storage,
            &remote,
            &ManifestUri {
                bucket: BUCKET.to_string(),
                namespace: namespace.clone(),
                hash: ours.clone(),
                origin: None,
            },
        )
        .await?;
        let domain_io = DomainLineageIo::new(paths.lineage());
        domain_io.write(&storage, domain_lineage).await?;
        let package = InstalledPackage {
            lineage: PackageLineageIo::new(domain_io, namespace.clone()),
            paths,
            remote: Arc::new(remote),
            storage,
            namespace,
            state_store: None,
        };
        package
            .install_paths(&[PathBuf::from("a.txt"), PathBuf::from("b.txt")])
            .await?;
        let (_, mut lineage) = package.lineage.read(&package.storage).await?;
        lineage.base_hash.clone_from(&base);
        lineage.latest_hash.clone_from(&merged);
        package.lineage.write(&package.storage, lineage).await?;

        Ok(Self {
            package,
            base,
            ours,
            merged,
            _dirs: [scratch, home_dir, paths_dir],
        })
    }

    async fn move_to_merged(&self) -> Res<flow::Pulled> {
        let target = ManifestUri {
            bucket: BUCKET.to_string(),
            namespace: self.package.namespace.clone(),
            hash: self.merged.clone(),
            origin: None,
        };
        let locked = self.package.lock().await?;
        locked.move_to(target, SyncScope::IndividualFiles).await
    }

    async fn path(&self, key: &str) -> Res<PathBuf> {
        Ok(self.package.package_home().await?.join(key))
    }

    async fn write(&self, key: &str, body: &[u8]) -> Res {
        tokio::fs::write(self.path(key).await?, body).await?;
        Ok(())
    }

    async fn read(&self, key: &str) -> Res<Vec<u8>> {
        Ok(tokio::fs::read(self.path(key).await?).await?)
    }

    /// The copy's own revision and its base.
    async fn revisions(&self) -> Res<(String, String)> {
        let lineage = self.package.lineage().await?;
        Ok((lineage.remote()?.hash.clone(), lineage.base_hash))
    }
}

fn assert_refused_at_b(moved: &Res<flow::Pulled>) {
    assert!(
        matches!(moved, Err(Error::PackageOp(PackageOpError::PullConflict(paths))) if paths == &vec![PathBuf::from("b.txt")]),
        "expected a conflict at b.txt, got: {moved:?}"
    );
}

fn keys(paths: &[PathBuf]) -> Vec<&Path> {
    paths.iter().map(PathBuf::as_path).collect()
}

/// An edit made while the merge ran, to the file the merge itself published,
/// stays as a change on top of the merged revision: the next publish carries
/// it, and nothing conflicts with the copy's own change.
#[test(tokio::test)]
async fn an_edit_to_a_path_the_merge_published_is_kept_on_top_of_it() -> Res {
    let copy = Diverged::new().await?;
    copy.write("a.txt", b"mine, edited").await?;

    let moved = copy.move_to_merged().await?;

    assert_eq!(copy.read("a.txt").await?, b"mine, edited");
    assert_eq!(copy.read("b.txt").await?, b"theirs");
    let flow::Pulled::Applied(report) = moved else {
        panic!("the move applied nothing");
    };
    assert_eq!(keys(&report.updated), [Path::new("b.txt")]);
    assert_eq!(
        copy.revisions().await?,
        (copy.merged.clone(), copy.merged.clone())
    );
    let status = copy.package.status(Some(HostConfig::default())).await?;
    assert_eq!(status.upstream_state, UpstreamState::UpToDate);
    assert_eq!(
        status.changes.keys().collect::<Vec<_>>(),
        [&PathBuf::from("a.txt")]
    );
    Ok(())
}

#[test(tokio::test)]
async fn an_edit_to_a_path_the_other_side_changed_refuses_the_move() -> Res {
    let copy = Diverged::new().await?;
    copy.write("b.txt", b"mine too").await?;

    assert_refused_at_b(&copy.move_to_merged().await);

    assert_eq!(copy.read("b.txt").await?, b"mine too");
    assert_eq!(
        copy.revisions().await?,
        (copy.ours.clone(), copy.base.clone()),
        "still Diverged at its own revision"
    );
    Ok(())
}

/// A save that lands after the move checked the folder, while it fetches
/// what it will write.
#[test(tokio::test)]
async fn a_save_while_the_move_fetches_is_not_overwritten() -> Res {
    let copy = Diverged::new().await?;
    let gate = copy
        .package
        .remote
        .park(&format!("s3://{BUCKET}/f/a/b.txt?versionId=b2"));
    let save = async {
        arrives(&gate, "the fetch of b.txt").await;
        copy.write("b.txt", b"saved meanwhile").await?;
        gate.release();
        Res::Ok(())
    };

    let (moved, saved) = tokio::join!(copy.move_to_merged(), save);
    saved?;

    assert_refused_at_b(&moved);
    assert_eq!(copy.read("b.txt").await?, b"saved meanwhile");
    Ok(())
}
