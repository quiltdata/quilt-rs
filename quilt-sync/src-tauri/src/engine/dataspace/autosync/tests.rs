use super::*;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::World;

/// The watcher queues a job for a change the tick's commit takes, and none
/// for one the contract's `[ignore]` table leaves out.
#[tokio::test]
async fn a_change_the_ignore_table_leaves_out_queues_nothing() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    ds.autosync_on().await?;
    let contract = std::fs::read_to_string(world.home().join("data.toml"))?;
    std::fs::write(
        world.home().join("data.toml"),
        format!("{contract}\n[ignore]\nextend_patterns = [\"*.tmp\"]\n"),
    )?;
    world.write("scratch.tmp", "ignored")?;
    world.write("a.txt", "v2")?;

    let ignored = changed(&ds, &world.folder().join("scratch.tmp")).await?;
    let taken = changed(&ds, &world.folder().join("a.txt")).await?;

    assert!(ignored.is_empty(), "{ignored:?}");
    assert_eq!(taken.len(), 1, "{taken:?}");
    Ok(())
}

/// With both directions off the runner does not tick; with one on, it
/// commits and pushes a changed folder.
#[tokio::test]
async fn the_runner_ticks_in_place_of_the_autopull_tick() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    ds.autosync_on().await?;
    world.write("a.txt", "v2")?;
    changed(&ds, &world.folder().join("a.txt")).await?;

    assert!(tick(&ds, false, false).await?.is_none());
    let ticked = tick(&ds, true, false).await?.ok_or("no tick")?;

    assert_eq!(ticked.dataspaces.len(), 1, "{ticked:?}");
    let status = ds
        .entry_status(&crate::engine::dataspace::tests::raw(), true)
        .await?;
    assert!(status.unpushed.is_empty(), "{status:?}");
    assert!(status.changes.is_empty(), "{status:?}");
    assert_eq!(status.hashes.latest, status.hashes.base);
    Ok(())
}
