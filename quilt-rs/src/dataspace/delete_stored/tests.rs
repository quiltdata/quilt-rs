use test_log::test;

use super::*;

use crate::dataspace::list::stored;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::status::tests::FIRST;
use crate::dataspace::status::tests::Res;
use crate::dataspace::status::tests::SECOND;
use crate::dataspace::status::tests::World;
use crate::dataspace::status::tests::package;
use crate::dataspace::store::tests::publish_local;

/// A top hash that shares [`FIRST`]'s first seven characters.
const NEAR_FIRST: &str = "1111111222222222222222222222222222222222222222222222222222222222";

fn place(path: &std::path::Path) -> Res {
    std::fs::create_dir_all(path.parent().ok_or("no folder")?)?;
    std::fs::write(path, "{}")?;
    Ok(())
}

fn bucket(name: &str) -> PackageKey {
    PackageKey {
        registry: Registry::S3(name.to_string()),
        package: package(),
    }
}

/// With no hash, the package leaves every store: each bucket's, unpushed
/// commits included, and the local registry, its `latest` too. The
/// dataspace's contract, lock and files are left.
#[test(tokio::test)]
async fn the_whole_package_leaves_every_store() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.commit().await?;
    place(
        &world
            .domain()
            .registry("c")?
            .installed_manifest(&package(), SECOND),
    )?;
    let local = world.local_named("lab-mac").await?;
    publish_local(&world.storage, &local, "team/raw", &[("o.txt", b"o1")]).await?;
    let contract = std::fs::read(world.dataspace().join("data.toml"))?;
    let lock = std::fs::read(world.dataspace().join(LOCK_FILE))?;

    let deleted = delete_stored(&world.storage, &world.domain(), &package(), None).await?;

    let mut keys: Vec<PackageKey> = deleted.iter().map(|store| store.key.clone()).collect();
    keys.sort();
    let local_key = PackageKey {
        registry: Registry::Local("lab-mac".to_string()),
        package: package(),
    };
    let mut expected = vec![bucket("b"), bucket("c"), local_key.clone()];
    expected.sort();
    assert_eq!(keys, expected);
    let in_b = deleted
        .iter()
        .find(|store| store.key == bucket("b"))
        .ok_or("b")?;
    assert_eq!(in_b.revisions.len(), 1, "the unpushed commit: {in_b:?}");
    let in_local = deleted
        .iter()
        .find(|store| store.key == local_key)
        .ok_or("local")?;
    assert_eq!(in_local.latest, LatestAfter::Removed);
    assert_eq!(stored(&world.storage, &world.domain()).await?, []);
    assert_eq!(local.read_latest(&world.storage, &package()).await?, None);
    assert_eq!(
        std::fs::read(world.dataspace().join("data.toml"))?,
        contract
    );
    assert_eq!(std::fs::read(world.dataspace().join(LOCK_FILE))?, lock);
    assert_eq!(std::fs::read_to_string(world.folder().join("a.txt"))?, "v2");
    Ok(())
}

/// A prefix removes only the revision it names, and the rest stay; a
/// store not holding it is not reported.
#[test(tokio::test)]
async fn a_prefix_removes_only_its_revision() -> Res {
    let world = World::synced()?;
    let store = world.domain().registry("b")?;
    place(&store.installed_manifest(&package(), FIRST))?;
    place(&store.installed_manifest(&package(), SECOND))?;
    let other = world
        .domain()
        .registry("c")?
        .installed_manifest(&package(), FIRST);
    place(&other)?;

    let deleted =
        delete_stored(&world.storage, &world.domain(), &package(), Some("2222222")).await?;

    assert_eq!(
        deleted,
        [StoreDeleted {
            key: bucket("b"),
            revisions: vec![SECOND.to_string()],
            latest: LatestAfter::Kept,
        }]
    );
    assert!(store.installed_manifest(&package(), FIRST).exists());
    assert!(!store.installed_manifest(&package(), SECOND).exists());
    assert!(other.exists());
    Ok(())
}

/// One hash held by two stores is no ambiguity: it leaves both.
#[test(tokio::test)]
async fn one_hash_leaves_every_store_holding_it() -> Res {
    let world = World::synced()?;
    for name in ["b", "c"] {
        place(
            &world
                .domain()
                .registry(name)?
                .installed_manifest(&package(), FIRST),
        )?;
    }

    let deleted = delete_stored(&world.storage, &world.domain(), &package(), Some(FIRST)).await?;

    assert_eq!(deleted.len(), 2, "{deleted:?}");
    assert_eq!(stored(&world.storage, &world.domain()).await?, []);
    Ok(())
}

/// A prefix two stores resolve to different revisions is refused, and
/// nothing is removed.
#[test(tokio::test)]
async fn a_prefix_ambiguous_across_stores_removes_nothing() -> Res {
    let world = World::synced()?;
    let b = world
        .domain()
        .registry("b")?
        .installed_manifest(&package(), FIRST);
    let c = world
        .domain()
        .registry("c")?
        .installed_manifest(&package(), NEAR_FIRST);
    place(&b)?;
    place(&c)?;

    let err = delete_stored(&world.storage, &world.domain(), &package(), Some("1111111"))
        .await
        .expect_err("ambiguous");

    assert!(
        matches!(&err, DeleteError::HashPrefix(HashPrefixError::Ambiguous { matches, more: 0, .. })
            if matches == &[FIRST.to_string(), NEAR_FIRST.to_string()]),
        "{err:?}"
    );
    assert!(b.exists() && c.exists());
    Ok(())
}

/// A prefix none of the package's revisions starts is refused.
#[test(tokio::test)]
async fn a_prefix_matching_nothing_is_refused() -> Res {
    let world = World::synced()?;
    let held = world
        .domain()
        .registry("b")?
        .installed_manifest(&package(), FIRST);
    place(&held)?;

    let err = delete_stored(&world.storage, &world.domain(), &package(), Some("2222222"))
        .await
        .expect_err("no match");

    assert!(
        matches!(err, DeleteError::HashPrefix(HashPrefixError::NoMatch(_))),
        "{err:?}"
    );
    assert!(held.exists());
    Ok(())
}

/// In the local registry, removing the revision `latest` names moves it to
/// the newest left, and removing the last removes it and the package.
#[test(tokio::test)]
async fn the_local_latest_moves_to_the_newest_left() -> Res {
    let world = World::synced()?;
    let local = world.local_named("lab-mac").await?;
    let first = publish_local(&world.storage, &local, "team/raw", &[("a.txt", b"v1")]).await?;
    let second = publish_local(&world.storage, &local, "team/raw", &[("a.txt", b"v2")]).await?;

    let moved = delete_stored(
        &world.storage,
        &world.domain(),
        &package(),
        Some(&second[..10]),
    )
    .await?;
    let gone = delete_stored(
        &world.storage,
        &world.domain(),
        &package(),
        Some(&first[..10]),
    )
    .await?;

    assert_eq!(moved[0].latest, LatestAfter::Moved(first.clone()));
    assert_eq!(gone[0].latest, LatestAfter::Removed);
    assert!(!local.packages(&world.storage).await?.contains(&package()));
    Ok(())
}

/// A package no store holds is refused by its name.
#[test(tokio::test)]
async fn a_package_no_store_holds_is_refused() -> Res {
    let world = World::synced()?;

    let err = delete_stored(&world.storage, &world.domain(), &package(), None)
        .await
        .expect_err("not held");

    assert!(
        matches!(&err, DeleteError::NotFound(namespace) if **namespace == package()),
        "{err:?}"
    );
    Ok(())
}

/// A package another process holds the lock of in any store is refused,
/// and nothing is removed.
#[test(tokio::test)]
async fn a_busy_package_removes_nothing() -> Res {
    let world = World::synced()?;
    let store = world.domain().registry("b")?;
    let held = store.installed_manifest(&package(), FIRST);
    place(&held)?;
    let _other = world
        .storage
        .try_lock_exclusive(store.package_lock(&package()))
        .await?
        .ok_or("not locked")?;

    let err = delete_stored(&world.storage, &world.domain(), &package(), None)
        .await
        .expect_err("busy");

    assert!(
        matches!(&err, DeleteError::Busy(key) if **key == bucket("b")),
        "{err:?}"
    );
    assert!(held.exists());
    Ok(())
}
