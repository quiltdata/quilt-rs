//! Validation item 4 of <https://github.com/quiltdata/quilt-specs/pull/292>:
//! processes writing one package, or different packages, at once.

use std::collections::BTreeSet;
use std::process::Child;

use test_log::test;

use super::*;
use crate::io::storage::Storage;

/// How many processes write at once.
const WRITERS: usize = 4;

/// Writers, each killed and reaped if the test ends before it does.
struct Writers(Vec<Child>);

impl Writers {
    /// Waits up to [`LIMIT`] for every writer, then checks each exited 0.
    fn finish(mut self, store: LineageStore) -> Res {
        let deadline = Instant::now() + LIMIT;
        let mut statuses = Vec::new();
        for child in &mut self.0 {
            loop {
                if let Some(status) = child.try_wait()? {
                    statuses.push(status);
                    break;
                }
                assert!(Instant::now() < deadline, "{store:?}: a writer never ended");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        for status in statuses {
            assert!(status.success(), "{store:?}: a writer failed: {status}");
        }
        Ok(())
    }
}

impl Drop for Writers {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Whether `child` is still running after a moment.
fn waits(child: &mut Child) -> Res<bool> {
    std::thread::sleep(Duration::from_millis(500));
    Ok(child.try_wait()?.is_none())
}

/// Each process pulls the target, which is behind, and downloads a file of
/// its own: the first pull lands, and every download is tracked and on disk.
#[test(tokio::test)]
async fn processes_pulling_and_downloading_one_package_lose_no_update() -> Res {
    for store in STORES {
        let dir = TempDir::new()?;
        let world = World::new(&dir, store).await?;
        let files: Vec<String> = (0..WRITERS).map(|n| format!("{n}.txt")).collect();
        let rows = |shared: &'static [u8]| {
            let mut rows: Vec<(&str, &[u8])> = vec![("shared.txt", shared)];
            rows.extend(files.iter().map(|file| (file.as_str(), file.as_bytes())));
            rows
        };
        let first = world.publish(&rows(b"one")).await?;
        world.domain().install_package(&first).await?;
        world
            .package()
            .install_paths(&["shared.txt".into()])
            .await?;
        let second = world.publish(&rows(b"two")).await?;

        let mut writers = Writers(Vec::new());
        for file in &files {
            writers
                .0
                .push(spawn(&world, Operation::PullAndDownload, file)?);
        }
        writers.finish(store)?;

        let lineage = world.package().lineage().await?;
        assert_eq!(lineage.base_hash, second.hash, "{store:?}");
        let tracked: BTreeSet<String> = lineage
            .paths
            .keys()
            .map(|key| key.to_string_lossy().replace('\\', "/"))
            .collect();
        for file in &files {
            assert_eq!(
                std::fs::read(world.working_file(file))?,
                file.as_bytes(),
                "{store:?}"
            );
        }
        let mut expected: BTreeSet<String> = files.into_iter().collect();
        expected.insert("shared.txt".to_string());
        assert_eq!(tracked, expected, "{store:?}");
        assert_eq!(
            std::fs::read(world.working_file("shared.txt"))?,
            b"two",
            "{store:?}"
        );
    }
    Ok(())
}

/// Each process creates a package of its own and commits to it: every
/// package keeps every commit. Not on `data.json`, where such a writer can
/// fail: `read_bytes` takes a file's length before it opens it, so a read
/// racing another package's rename of the file stops short.
#[test(tokio::test)]
async fn processes_creating_and_committing_different_packages_on_refs_lose_no_update() -> Res {
    let dir = TempDir::new()?;
    let world = World::new(&dir, LineageStore::Refs).await?;
    // Two owners, so creating a package creates its owner's folder in some
    // writers and finds it in others.
    let packages: Vec<Namespace> = (0..WRITERS)
        .map(|n| Namespace::from((format!("team{}", n % 2), format!("p{n}"))))
        .collect();
    let mut writers = Writers(Vec::new());
    for namespace in &packages {
        writers.0.push(spawn(
            &world,
            Operation::CreateAndCommit,
            &namespace.to_string(),
        )?);
    }
    writers.finish(LineageStore::Refs)?;

    let read = world.domain().read_packages().await?;
    let committed: Vec<usize> = packages
        .iter()
        .map(|namespace| {
            read.get(namespace)
                .and_then(|entry| entry.as_ref().ok()?.commit.as_ref())
                .map_or(0, |commit| commit.prev_hashes.len())
        })
        .collect();
    // Each commit after the one creating the package keeps the one before it.
    assert!(read.values().all(Result::is_ok), "{read:?}");
    assert_eq!(read.len(), WRITERS + 1, "with the bystander");
    assert_eq!(committed, vec![COMMITS; WRITERS]);
    Ok(())
}

/// On `data.json` a commit waits on the short lock every package's writer
/// takes. On refs it takes no lock another package's writer takes: only
/// creating a package waits, on the domain lock.
#[test(tokio::test)]
async fn a_commit_waits_on_a_domain_wide_lock_only_on_data_json() -> Res {
    for store in STORES {
        let dir = TempDir::new()?;
        let world = World::new(&dir, store).await?;
        let first = world.publish(&[("a.txt", b"one")]).await?;
        world.domain().install_package(&first).await?;
        std::fs::write(world.working_file("a.txt"), b"edit")?;

        let paths = DomainPaths::new(world.root.join("domain"));
        let lock = match store {
            LineageStore::DataJson => paths.lineage_lock(),
            LineageStore::Refs => paths.domain_file_lock(),
        };
        let held = LocalStorage::new().lock_exclusive(&lock).await?;
        let mut commit = Writers(vec![spawn(&world, Operation::Commit, "")?]);
        if store == LineageStore::DataJson {
            assert!(waits(&mut commit.0[0])?, "the commit waits");
            assert!(world.package().lineage().await?.commit.is_none());
            drop(held);
            commit.finish(store)?;
        } else {
            commit.finish(store)?;
            let mut create = Writers(vec![spawn(&world, Operation::CreateAndCommit, "acme/new")?]);
            assert!(waits(&mut create.0[0])?, "creating a package waits");
            let new = Namespace::from(("acme", "new"));
            assert!(!world.domain().read_packages().await?.contains_key(&new));
            drop(held);
            create.finish(store)?;
            let created = world.domain().read_packages().await?.remove(&new);
            let commits = created
                .and_then(|entry| entry.ok()?.commit)
                .map(|commit| commit.prev_hashes.len());
            assert_eq!(commits, Some(COMMITS), "the create lands once let go");
        }
        assert!(
            world.package().lineage().await?.commit.is_some(),
            "{store:?}"
        );
    }
    Ok(())
}
