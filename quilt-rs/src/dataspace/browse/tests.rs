use test_log::test;

use super::*;

use crate::dataspace::status::tests::FIRST;
use crate::dataspace::status::tests::Of;
use crate::dataspace::status::tests::Res;
use crate::dataspace::status::tests::World;
use crate::dataspace::status::tests::key;
use crate::dataspace::status::tests::writer;

async fn opened(world: &World, package: &PackageKey) -> Result<Browsed, BrowseError> {
    browse(
        &world.storage,
        &world.remote,
        &world.domain(),
        &world.dataspace(),
        &writer(),
        package,
    )
    .await
}

fn keys(browsed: &Browsed) -> Of<Vec<String>> {
    Ok(browsed
        .manifest
        .rows
        .iter()
        .map(|row| row.logical_key.to_string())
        .collect())
}

/// With no local commit, the package opens at its base.
#[test(tokio::test)]
async fn a_package_opens_at_its_base() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;

    let browsed = opened(&world, &key()).await?;

    assert_eq!(browsed.hash, FIRST);
    assert_eq!(browsed.entry.key(), key());
    assert_eq!(keys(&browsed)?, ["a.txt"]);
    Ok(())
}

/// A local commit the bucket has not had is what opens, not the base.
#[test(tokio::test)]
async fn a_package_opens_at_its_last_local_commit() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("b.txt", "new")?;
    world.commit().await?;
    let committed = world
        .lock()
        .await?
        .packages
        .get(&key())
        .and_then(|locked| locked.commits.last().cloned())
        .ok_or("no commit")?;

    let browsed = opened(&world, &key()).await?;

    assert_eq!(browsed.hash, committed);
    assert_ne!(browsed.hash, FIRST);
    assert_eq!(keys(&browsed)?, ["a.txt", "b.txt"]);
    Ok(())
}

/// A package the contract does not declare is refused, by its key.
#[test(tokio::test)]
async fn an_undeclared_package_is_refused() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    let other = PackageKey {
        registry: Registry::S3("b".to_string()),
        package: ("team", "other").into(),
    };

    let err = opened(&world, &other).await.expect_err("undeclared");

    assert!(
        matches!(&err, BrowseError::NotDeclared(refused) if **refused == other),
        "{err:?}"
    );
    Ok(())
}

/// A declared package the lock holds no revision of has nothing to open.
#[test(tokio::test)]
async fn a_package_never_read_has_nothing_to_open() -> Res {
    let world = World::synced()?;

    let err = opened(&world, &key()).await.expect_err("never read");

    assert!(
        matches!(&err, BrowseError::NotRead(refused) if **refused == key()),
        "{err:?}"
    );
    Ok(())
}

/// Browsing writes nothing to the dataspace: its lock is as it was.
#[test(tokio::test)]
async fn browsing_writes_nothing_to_the_dataspace() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    let lock = world.dataspace().join(crate::dataspace::lock::LOCK_FILE);
    let before = std::fs::read(&lock)?;

    opened(&world, &key()).await?;

    assert_eq!(std::fs::read(&lock)?, before);
    Ok(())
}

/// A package `data.toml` moved to its bucket, whose move has not landed,
/// opens at its last commit in the local registry, under its bucket's key.
#[test(tokio::test)]
async fn a_package_moved_to_its_bucket_opens_where_its_commits_are() -> Res {
    let (world, _, head) = crate::dataspace::sync::tests::moving().await?;
    world.local_named("lab-mac").await?;
    let bucket = crate::dataspace::sync::tests::key();

    let browsed = browse(
        &world.storage,
        &world.remote,
        &world.domain(),
        &world.dataspace(),
        &crate::dataspace::sync::tests::writer(),
        &bucket,
    )
    .await?;

    assert_eq!(browsed.hash, head);
    assert_eq!(browsed.entry.key(), bucket);
    Ok(())
}
