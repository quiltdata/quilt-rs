use test_log::test;

use super::*;

use crate::Error;
use crate::dataspace::status::tests::FIRST;
use crate::dataspace::status::tests::Res;
use crate::dataspace::status::tests::SECOND;
use crate::dataspace::status::tests::THIRD;
use crate::dataspace::status::tests::World;
use crate::dataspace::status::tests::key;
use crate::dataspace::status::tests::package;
use crate::dataspace::store::tests::publish_local;
use crate::error::HashPrefixError;

fn place(path: &std::path::Path) -> Res {
    std::fs::create_dir_all(path.parent().ok_or("no folder")?)?;
    std::fs::write(path, "{}")?;
    Ok(())
}

/// A prefix resolves among the package's installed manifests in its
/// bucket's store.
#[test(tokio::test)]
async fn a_prefix_resolves_in_the_buckets_store() -> Res {
    let world = World::synced()?;
    let store = world.domain().registry("b")?;
    place(&store.installed_manifest(&package(), FIRST))?;
    place(&store.installed_manifest(&package(), SECOND))?;

    let hash = resolve_hash(&world.storage, &world.domain(), &key(), "2222222").await?;

    assert_eq!(hash, SECOND);
    Ok(())
}

/// A manifest fetched from the bucket into `packages/` is not one of the
/// package's revisions here (entry 27d).
#[test(tokio::test)]
async fn a_fetched_manifest_is_not_a_revision() -> Res {
    let world = World::synced()?;
    place(&world.domain().registry("b")?.cached_manifest(THIRD))?;

    let err = resolve_hash(&world.storage, &world.domain(), &key(), "3333333")
        .await
        .expect_err("no revision");

    assert!(
        matches!(err, Error::HashPrefix(HashPrefixError::NoMatch(_))),
        "{err:?}"
    );
    Ok(())
}

/// The local registry's revisions resolve by its name, and another
/// registry's name finds none of them.
#[test(tokio::test)]
async fn the_local_registry_resolves_by_its_name() -> Res {
    let world = World::synced()?;
    let local = world.local_named("lab-mac").await?;
    let out = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    let named = |name: &str| PackageKey {
        registry: Registry::Local(name.to_string()),
        package: ("me", "out").into(),
    };

    let hash = resolve_hash(
        &world.storage,
        &world.domain(),
        &named("lab-mac"),
        &out[..8],
    )
    .await?;
    let err = resolve_hash(
        &world.storage,
        &world.domain(),
        &named("elsewhere"),
        &out[..8],
    )
    .await
    .expect_err("another registry");

    assert_eq!(hash, out);
    assert!(
        matches!(err, Error::HashPrefix(HashPrefixError::NoMatch(_))),
        "{err:?}"
    );
    Ok(())
}

/// A prefix too short is refused as such, even for a registry not held.
#[test(tokio::test)]
async fn a_short_prefix_is_refused() -> Res {
    let world = World::synced()?;
    let elsewhere = PackageKey {
        registry: Registry::Local("elsewhere".to_string()),
        package: package(),
    };

    let err = resolve_hash(&world.storage, &world.domain(), &elsewhere, "12")
        .await
        .expect_err("too short");

    assert!(
        matches!(err, Error::HashPrefix(HashPrefixError::InvalidLength(_))),
        "{err:?}"
    );
    Ok(())
}
