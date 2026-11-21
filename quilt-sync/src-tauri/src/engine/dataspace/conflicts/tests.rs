use quilt_rs::dataspace::install::Install;

use super::*;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::URI;
use crate::engine::dataspace::tests::World;
use crate::engine::dataspace::tests::raw;

/// `team/raw` with `a.txt` changed here and, in a newer revision, there:
/// the sync here is blocked, and `conflict.json` holds `a.txt`.
async fn colliding()
-> Result<(World, Dataspace<quilt_rs::io::remote::mocks::MockRemote>), Box<dyn std::error::Error>> {
    let world = World::new()?;
    let ds = world.published().await?;
    let there = world.elsewhere()?;
    let theirs = there.open().await?;
    theirs.install(&Install::new(URI)).await?;
    there.write("a.txt", "theirs")?;
    // The first sync of a dataspace only reads, until it is filled.
    theirs.sync("filled").await?;
    theirs.sync("theirs").await?;
    world.write("a.txt", "mine")?;
    ds.sync("mine").await?;
    Ok((world, ds))
}

/// A read blocked by local changes shows its row, and deciding it for the
/// other side writes the decision the next sync takes.
#[tokio::test]
async fn a_conflict_is_shown_and_decided() -> Res {
    let (world, ds) = colliding().await?;

    let listed = rows(&ds).await?;
    let [row] = listed.as_slice() else {
        panic!("{listed:?}");
    };
    assert_eq!(row.namespace, raw());
    assert_eq!(row.key, "a.txt");
    assert_eq!(row.kind, ConflictKind::Read);
    assert!(row.mine && row.theirs, "{row:?}");
    assert_eq!(row.decision, None);

    decide(&ds, &row.package, "a.txt", Decision::Theirs).await?;
    assert_eq!(rows(&ds).await?[0].decision, Some(Decision::Theirs));
    ds.sync("decided").await?;

    assert_eq!(world.text("a.txt").as_deref(), Some("theirs"));
    assert_eq!(rows(&ds).await?, []);
    Ok(())
}

/// Keeping the folder's file is decided by naming it, and taken too.
#[tokio::test]
async fn keeping_mine_keeps_the_folders_file() -> Res {
    let (world, ds) = colliding().await?;
    let package = rows(&ds).await?[0].package.clone();

    decide(&ds, &package, "a.txt", Decision::Mine).await?;
    ds.sync("decided").await?;

    assert_eq!(world.text("a.txt").as_deref(), Some("mine"));
    assert_eq!(rows(&ds).await?, []);
    Ok(())
}
