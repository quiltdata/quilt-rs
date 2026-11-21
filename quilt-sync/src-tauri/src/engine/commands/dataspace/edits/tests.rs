use std::str::FromStr;

use quilt_rs::dataspace::create::Create;
use quilt_uri::Host;
use quilt_uri::Namespace;

use super::*;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::World;
use crate::engine::dataspace::tests::raw;

/// Setting a remote on a package of the local registry moves it to the
/// bucket in `data.toml`, its folder where it was; once a package is
/// published there, its registry is the contract's to change.
#[tokio::test]
async fn setting_a_remote_moves_a_local_package_to_the_bucket() -> Res {
    let world = World::new()?;
    let ds = world.open().await?;
    let name = ds.local_name().await?;
    let folder = world.home().join("mine");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("a.txt"), "v1")?;
    let mut request = Create::new(format!("quilt+local://{name}#package=me/mine"));
    request.folder = Some(folder.clone());
    ds.create(&request).await?;
    let mine = Namespace::from(("me", "mine"));

    set_remote(&ds, "me/mine", "example.quiltdata.com", "b2").await?;

    let entry = ds.entry(&mine).await?;
    assert_eq!(entry.registry, Registry::S3("b2".to_string()));
    assert_eq!(
        entry.catalog,
        Some(Host::from_str("example.quiltdata.com")?)
    );
    assert_eq!(entry.folder, folder);
    // The next sync finds the move and publishes it (25.10).
    ds.sync("moved").await?;
    let moved = ds.entry_status(&mine, true).await?;
    assert!(moved.hashes.base.is_some(), "{moved:?}");
    assert!(moved.unpushed.is_empty(), "{moved:?}");

    let published = world.published().await?;
    let refused = set_remote(&published, "team/raw", "", "b3").await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(
        published.entry(&raw()).await?.registry,
        Registry::S3("b".to_string())
    );
    Ok(())
}

/// The catalog is set, also once published, and a blank one takes it off.
#[tokio::test]
async fn the_catalog_is_set_and_taken_off() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;

    change_catalog(&ds, "team/raw", "example.quiltdata.com").await?;
    assert_eq!(
        ds.entry(&raw()).await?.catalog,
        Some(Host::from_str("example.quiltdata.com")?)
    );
    change_catalog(&ds, "team/raw", " ").await?;
    assert_eq!(ds.entry(&raw()).await?.catalog, None);
    Ok(())
}

/// A pattern goes to the folder's own `.quiltignore`; stopping to ignore a
/// file writes its `!` line and counts what comes back; a nested dataspace
/// is never brought back.
#[tokio::test]
async fn the_folder_rules_are_edited() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    world.write("x.log", "noise")?;

    add_to_quiltignore(&ds, "team/raw", "*.log\nsecond line").await?;
    assert_eq!(world.text(".quiltignore").as_deref(), Some("*.log\n"));

    let preview = stop_ignoring_preview(&ds, "team/raw", "x.log", false).await?;
    assert_eq!(preview.this_package.files, 1);
    assert_eq!(preview.this_package.bytes, 5);
    let lines = stop_ignoring(&ds, "team/raw", "x.log", false, Scope::Package).await?;
    assert_eq!(lines, Some(vec!["!/x.log".to_string()]));
    assert!(
        world
            .text(".quiltignore")
            .is_some_and(|text| text.contains("!/x.log"))
    );

    world.write("inner/data.toml", "")?;
    let refused = stop_ignoring_preview(&ds, "team/raw", "inner", true).await;
    assert!(
        matches!(&refused, Err(Error::General(text)) if text.contains("another dataspace")),
        "{refused:?}"
    );
    Ok(())
}

/// The local registry is renamed; its new name is the one it gives.
#[tokio::test]
async fn the_local_registry_is_renamed() -> Res {
    let world = World::new()?;
    let ds = world.open().await?;

    assert_eq!(rename(&ds, " lab-mac ").await?, "lab-mac");
    assert_eq!(ds.local_name().await?, "lab-mac");
    Ok(())
}
