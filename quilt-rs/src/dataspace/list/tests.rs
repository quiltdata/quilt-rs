use test_log::test;

use super::*;

use crate::dataspace::status::tests::FIRST;
use crate::dataspace::status::tests::Res;
use crate::dataspace::status::tests::SECOND;
use crate::dataspace::status::tests::THIRD;
use crate::dataspace::status::tests::World;
use crate::dataspace::status::tests::package;
use crate::dataspace::status::tests::writer;
use crate::dataspace::store::tests::publish_local;

/// The union of every store: a package committed in one bucket's store, one
/// installed in another's, and one the local registry holds, under its name.
#[test(tokio::test)]
async fn the_list_unions_the_local_stores() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.commit().await?;
    let other: Namespace = ("team", "other").into();
    let installed = world
        .domain()
        .registry("c")?
        .installed_manifest(&other, THIRD);
    std::fs::create_dir_all(installed.parent().ok_or("no folder")?)?;
    std::fs::write(&installed, "{}")?;
    let local = world.local_named("lab-mac").await?;
    let out = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;

    let mut listed = stored(&world.storage, &world.domain()).await?;

    listed.sort_by(|a, b| a.key.cmp(&b.key));
    let key = |registry: Registry, package: &str| -> Result<PackageKey, quilt_uri::UriError> {
        Ok(PackageKey {
            registry,
            package: Namespace::try_from(package)?,
        })
    };
    let mut expected = vec![
        StoredPackage {
            key: key(Registry::S3("b".to_string()), "team/raw")?,
            latest: None,
        },
        StoredPackage {
            key: key(Registry::S3("c".to_string()), "team/other")?,
            latest: None,
        },
        StoredPackage {
            key: key(Registry::Local("lab-mac".to_string()), "me/out")?,
            latest: Some(out),
        },
    ];
    expected.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(listed, expected);
    Ok(())
}

/// A local registry never named holds nothing, and listing the stores does
/// not name it.
#[test(tokio::test)]
async fn listing_the_stores_does_not_name_the_local_registry() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;

    stored(&world.storage, &world.domain()).await?;

    assert!(!world.domain().local().dir().join("registry.json").exists());
    Ok(())
}

/// A dataspace's list summarises each entry it declares.
#[test(tokio::test)]
async fn a_dataspace_list_summarises_each_entry() -> Res {
    let world = World::new("sync = [\"team/raw\"]\nread = [\"team/other\"]")?;
    world
        .publish_to("team/other", THIRD, &[("o.txt", b"o1")], 1_700_000_000)
        .await?;
    crate::dataspace::read_all::read_all(
        &world.storage,
        &world.remote,
        None,
        &world.dataspace(),
        &writer(),
    )
    .await?;
    world.fill(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;

    let listed = list(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        Some(&world.queue().await?),
        &world.dataspace(),
        &writer(),
        &world.user_rules(),
        true,
    )
    .await?;

    let states: Vec<(String, PackageState)> = listed
        .iter()
        .map(|declared| (declared.entry.package.to_string(), declared.state))
        .collect();
    assert_eq!(
        states,
        [
            ("team/other".to_string(), PackageState::Latest),
            (
                "team/raw".to_string(),
                PackageState::PendingChanges { files: 1 }
            ),
        ]
    );
    Ok(())
}

/// A registry's packages come from its latest index, which `fetch` refreshes
/// from the bucket first; the local registry's from its own tags.
#[test(tokio::test)]
async fn a_registry_lists_its_packages_from_its_latest_index() -> Res {
    let world = World::synced()?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    let local = world.local_named("lab-mac").await?;
    let out = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    let bucket = Registry::S3("b".to_string());
    let domain = world.domain();
    let listed = |registry: Registry, fetch: bool| {
        let domain = domain.clone();
        let world = &world;
        async move {
            registry_packages(
                &world.storage,
                &world.remote,
                &domain,
                &registry,
                None,
                fetch,
            )
            .await
        }
    };

    assert_eq!(listed(bucket.clone(), false).await?, BTreeMap::new());
    let fetched = listed(bucket.clone(), true).await?;
    assert_eq!(fetched, BTreeMap::from([(package(), FIRST.to_string())]));
    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    assert_eq!(
        listed(bucket, false).await?,
        fetched,
        "the index, not asked again"
    );

    let mine = listed(Registry::Local("lab-mac".to_string()), false).await?;
    assert_eq!(
        mine,
        BTreeMap::from([(Namespace::try_from("me/out")?, out)])
    );
    assert!(matches!(
        listed(Registry::Local("elsewhere".to_string()), false).await,
        Err(EntryError::LocalRegistry)
    ));
    Ok(())
}
