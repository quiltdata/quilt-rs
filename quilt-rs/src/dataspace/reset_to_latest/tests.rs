use super::*;

use test_log::test;

use crate::dataspace::METADATA_FILE;
use crate::dataspace::read::STAGING_PREFIX;
use crate::dataspace::reattach::unattached;
use crate::dataspace::store::tests::publish_local;
use crate::dataspace::sync::tests::FIRST;
use crate::dataspace::sync::tests::SECOND;
use crate::dataspace::sync::tests::World;
use crate::dataspace::sync::tests::key;
use crate::dataspace::sync::tests::out_key;
use crate::dataspace::sync::tests::read_text;
use crate::dataspace::sync::tests::writer;
use crate::failpoint::stopping_at;

type Res = Result<(), Box<dyn std::error::Error>>;

impl World {
    /// Resets `package`, as `quilt reset --dataspace` does.
    async fn reset_as(&self, package: &PackageKey) -> Result<Reset, ResetError> {
        reset_to_latest(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.dataspace(),
            &writer(),
            package,
        )
        .await
    }
}

/// Reset discards local commits and changes and reads `latest` whole; the
/// commits stay in the store, and a file never committed is left.
#[test(tokio::test)]
async fn reset_discards_commits_and_changes_and_reads_latest_whole() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.write("e.txt", "committed")?;
    let commit = world.commit("discarded").await?;
    world.remove("b.txt")?;
    world.write("a.txt", "v3")?;
    world.write("mine.txt", "never committed")?;
    world
        .publish(
            SECOND,
            &[("a.txt", b"v1"), ("b.txt", b"v2"), ("d.txt", b"d")],
        )
        .await?;

    let reset = world.reset_as(&key()).await?;

    assert_eq!(reset.revision, SECOND);
    assert_eq!(reset.discarded, std::slice::from_ref(&commit));
    assert_eq!(world.locked().await?, Some(LockEntry::at(SECOND)));
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("d.txt").as_deref(), Some("d"));
    assert_eq!(world.text("e.txt"), None, "the discarded commit's file");
    assert_eq!(world.text("mine.txt").as_deref(), Some("never committed"));
    assert!(world.text(METADATA_FILE).is_some());
    let listed = unattached(
        &world.storage,
        &world.remote,
        &world.domain(),
        &world.dataspace(),
        &writer(),
        &key(),
    )
    .await?;
    assert_eq!(
        listed
            .into_iter()
            .map(|commit| commit.hash)
            .collect::<Vec<_>>(),
        [commit]
    );
    // Nothing left to publish but the file never committed.
    let leftover: Vec<_> = std::fs::read_dir(world.folder())?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(STAGING_PREFIX))
        .collect();
    assert_eq!(leftover, Vec::<String>::new());
    Ok(())
}

#[test(tokio::test)]
async fn reset_at_latest_discards_the_local_changes() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.remove("b.txt")?;
    std::fs::write(world.folder().join(METADATA_FILE), "[metadata]\n")?;

    let reset = world.reset_as(&key()).await?;

    assert_eq!(reset.discarded, Vec::<String>::new());
    assert_eq!(world.locked().await?, Some(LockEntry::at(FIRST)));
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_ne!(world.text(METADATA_FILE).as_deref(), Some("[metadata]\n"));
    Ok(())
}

/// A reset stopped part-way puts back every file it moved, and leaves the
/// lock as it was.
#[test(tokio::test)]
async fn a_reset_stopped_part_way_puts_the_folder_back() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let commit = world.commit("kept").await?;
    world.write("b.txt", "edited")?;
    world
        .publish(SECOND, &[("a.txt", b"v9"), ("b.txt", b"v9")])
        .await?;
    let before = world.lock().await?;

    let stopped = stopping_at("replace.placed", 0, world.reset_as(&key())).await;

    assert!(stopped.is_err(), "{stopped:?}");
    assert_eq!(world.lock().await?, before);
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("b.txt").as_deref(), Some("edited"));
    assert_eq!(
        world.locked().await?.map(|locked| locked.commits),
        Some(vec![commit])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_read_entry_is_reset_to_latest() -> Res {
    let world = World::new("read = [\"team/raw\"]")?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.read_all().await?;
    world.write("a.txt", "mine")?;

    world.reset_as(&key()).await?;

    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.locked().await?, Some(LockEntry::at(FIRST)));
    Ok(())
}

#[test(tokio::test)]
async fn a_local_registry_entry_is_reset_to_its_latest() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    world.sync().await?;
    let folder = world.local_folder().await?;
    std::fs::write(folder.join("o.txt"), "o2")?;
    world.commit_as(&out_key("lab-mac"), "local edit").await?;
    let second = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o3")]).await?;

    let reset = world.reset_as(&out_key("lab-mac")).await?;

    assert_eq!(reset.revision, second);
    assert_eq!(read_text(&folder, "o.txt").as_deref(), Some("o3"));
    let locked = world
        .lock()
        .await?
        .packages
        .get(&out_key("lab-mac"))
        .cloned();
    assert_eq!(locked, Some(LockEntry::at(second)));
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_no_latest_or_not_read_is_refused() -> Res {
    let world = World::synced()?;
    world.write("a.txt", "first")?;
    let refused = world.reset_as(&key()).await;
    assert!(matches!(refused, Err(ResetError::NoLatest)), "{refused:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("first"));

    let world = World::new("write = [\"team/raw\"]")?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    let refused = world.reset_as(&key()).await;
    assert!(
        matches!(refused, Err(ResetError::Access(Access::Write))),
        "{refused:?}"
    );

    let world = World::new(&format!(
        "read = [{{ package = \"team/raw\", revision = \"{FIRST}\" }}]"
    ))?;
    let refused = world.reset_as(&key()).await;
    assert!(matches!(refused, Err(ResetError::Pinned(_))), "{refused:?}");
    Ok(())
}
