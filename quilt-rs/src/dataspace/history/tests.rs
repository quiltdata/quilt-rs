use test_log::test;

use super::*;

use crate::dataspace::status::tests::FIRST;
use crate::dataspace::status::tests::Of;
use crate::dataspace::status::tests::Res;
use crate::dataspace::status::tests::SECOND;
use crate::dataspace::status::tests::World;
use crate::dataspace::status::tests::key;
use crate::dataspace::status::tests::writer;
use crate::dataspace::store::tests::publish_local;
use crate::failpoint::stopping_at;
use quilt_uri::Namespace;

impl World {
    async fn history(&self, package: &PackageKey) -> Of<Vec<Revision>> {
        Ok(history(
            &self.storage,
            &self.remote,
            Some(&self.domain()),
            &self.dataspace(),
            &writer(),
            package,
        )
        .await?)
    }
}

/// The hashes, newest first, each marked `local` where it is a local commit.
fn listed(revisions: &[Revision]) -> Vec<(&str, bool)> {
    revisions
        .iter()
        .map(|revision| (revision.hash.as_str(), revision.local))
        .collect()
}

/// A bucket's history is its listing, newest first, with the commit the
/// lock names and the bucket lacks first, marked local and made on the base.
#[test(tokio::test)]
async fn a_bucket_history_marks_the_unpushed_commit_local() -> Res {
    let world = World::synced()?;
    world
        .publish_at(FIRST, &[("a.txt", b"v1")], 1_700_000_000)
        .await?;
    world
        .publish_at(SECOND, &[("a.txt", b"v2")], 1_700_000_060)
        .await?;
    world.sync().await?;
    world.write("a.txt", "mine")?;
    world.commit().await?;
    let lock = world.lock().await?;
    let commit = lock
        .packages
        .get(&key())
        .and_then(|locked| locked.commits.last().cloned())
        .ok_or("no commit")?;

    let revisions = world.history(&key()).await?;

    assert_eq!(
        listed(&revisions),
        [(commit.as_str(), true), (SECOND, false), (FIRST, false)]
    );
    assert_eq!(revisions[0].parent.as_deref(), Some(SECOND));
    assert_eq!(revisions[0].message.as_deref(), Some("autosync"));
    assert_eq!(
        revisions[1].message.as_deref(),
        Some("made 2222222222222222222222222222222222222222222222222222222222222222")
    );
    assert_eq!(
        revisions[1].time,
        Some(Timestamp::from_second(1_700_000_060)?)
    );
    assert_eq!(revisions[1].parent, None);

    // With no lock to say so, the store still holds the commit the bucket
    // lacks.
    let unlocked = revisions_of(&world, None).await?;
    assert_eq!(unlocked[0].hash, commit);
    assert!(
        unlocked[0].local && unlocked[0].parent.is_none(),
        "{unlocked:?}"
    );
    Ok(())
}

async fn revisions_of(world: &World, locked: Option<&LockEntry>) -> Of<Vec<Revision>> {
    Ok(revisions(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        &key(),
        None,
        locked,
    )
    .await?)
}

/// A local registry's history is what it published, with a commit made to
/// it and not published marked local.
#[test(tokio::test)]
async fn a_local_registry_history_marks_the_unpublished_commit_local() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    let first = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    let second = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o2")]).await?;
    world.sync().await?;
    let package = PackageKey {
        registry: Registry::Local("lab-mac".to_string()),
        package: Namespace::try_from("me/out")?,
    };
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let entry = contract.entries().first().ok_or("no entry")?;
    std::fs::write(entry.folder.join("p.txt"), "added")?;
    // The tick stops once the commit is recorded, before the lock names it
    // or it is published; the lock is then written as the tick would.
    stopping_at("sync.recorded", 0, world.sync()).await?;
    let dir = local.installed_manifests_dir(&package.package);
    let commit = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .find(|hash| *hash != first && *hash != second)
        .ok_or("no commit")?;
    let mut lock = world.lock().await?;
    lock.packages
        .get_mut(&package)
        .ok_or("no lock entry")?
        .commits
        .push(commit.clone());
    lock.write(&world.storage, &contract, &writer()).await?;

    let revisions = world.history(&package).await?;

    let mut marked = listed(&revisions);
    marked.sort_unstable();
    let mut expected = vec![
        (commit.as_str(), true),
        (first.as_str(), false),
        (second.as_str(), false),
    ];
    expected.sort_unstable();
    assert_eq!(marked, expected);
    let committed = revisions
        .iter()
        .find(|revision| revision.hash == commit)
        .ok_or("no commit")?;
    assert_eq!(committed.parent.as_deref(), Some(second.as_str()));
    assert!(
        revisions
            .windows(2)
            .all(|pair| pair[0].time >= pair[1].time),
        "{revisions:?}"
    );
    Ok(())
}

/// Commits only the lock names have no time, and are still the newest, the
/// last of them first.
#[test(tokio::test)]
async fn commits_only_the_lock_names_are_listed_newest_first() -> Res {
    let world = World::synced()?;
    world
        .publish_at(FIRST, &[("a.txt", b"v1")], 1_700_000_000)
        .await?;
    world.sync().await?;
    let (older, newer) = ("a".repeat(64), "b".repeat(64));
    let locked = LockEntry {
        base: Some(FIRST.to_string()),
        commits: vec![older.clone(), newer.clone()],
    };

    let revisions = revisions_of(&world, Some(&locked)).await?;

    assert_eq!(
        listed(&revisions),
        [
            (newer.as_str(), true),
            (older.as_str(), true),
            (FIRST, false)
        ]
    );
    Ok(())
}

/// A commit only the lock names has no time, and is still the newest.
#[test(tokio::test)]
async fn a_commit_only_the_lock_names_is_listed_first() -> Res {
    let world = World::synced()?;
    world
        .publish_at(FIRST, &[("a.txt", b"v1")], 1_700_000_000)
        .await?;
    world.sync().await?;
    let locked = LockEntry {
        base: Some(FIRST.to_string()),
        commits: vec![SECOND.to_string()],
    };

    let revisions = revisions_of(&world, Some(&locked)).await?;

    assert_eq!(listed(&revisions), [(SECOND, true), (FIRST, false)]);
    Ok(())
}

/// Commits with no base: the first has no parent, each later one the commit
/// before it, and commits nothing else lists are added as local.
#[test]
fn commits_with_no_base_are_made_each_on_the_one_before() {
    let locked = LockEntry {
        base: None,
        commits: vec![FIRST.to_string(), SECOND.to_string()],
    };
    let mut listed = Vec::new();

    follow_lock(&mut listed, &locked);

    let parents: Vec<(&str, Option<&str>, bool)> = listed
        .iter()
        .map(|revision| {
            (
                revision.hash.as_str(),
                revision.parent.as_deref(),
                revision.local,
            )
        })
        .collect();
    assert_eq!(parents, [(SECOND, Some(FIRST), true), (FIRST, None, true)]);
}
