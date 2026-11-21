use quilt_rs::dataspace::contract::PackagePath;
use quilt_rs::dataspace::contract::Paths;

use super::*;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::World;
use crate::engine::dataspace::tests::raw;

/// A sync whose read left files out did not complete, so a hand pull or
/// publish under `sync` does not report success; a whole read does.
#[tokio::test]
async fn an_incomplete_read_is_no_success() -> Res {
    use quilt_rs::dataspace::read::ReadReport;

    let world = World::new()?;
    let ds = world.published().await?;
    let entry = ds.entry(&raw()).await?;
    let whole = ReadReport::default();
    assert!(settled(&entry, &EntrySync::ReadBack(whole.clone())).is_ok());

    let mut partial = whole;
    partial.skipped.push(PathBuf::from("a.txt"));
    let ran = EntrySync::Ran {
        read: Some(partial.clone()),
        published: None,
    };
    assert!(
        matches!(settled(&entry, &ran), Err(Error::Dataspace(text)) if text.contains("not read whole")),
        "{ran:?}"
    );
    assert!(settled(&entry, &EntrySync::ReadBack(partial)).is_err());
    Ok(())
}

/// The main page lists what the default dataspace declares, each row with
/// the dataspace's own verdict.
#[tokio::test]
async fn the_main_page_lists_the_dataspace_packages() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;

    let listed = main_page_packages(&ds).await?;
    let [row] = listed.packages.as_slice() else {
        panic!("{listed:?}");
    };
    assert_eq!(row.namespace, raw());
    assert_eq!(row.bucket.as_deref(), Some("b"));
    assert_eq!(row.state, PackageStateDto::Latest);

    world.write("a.txt", "v2")?;
    let listed = main_page_packages(&ds).await?;
    assert_eq!(
        listed.packages[0].state,
        PackageStateDto::PendingChanges { files: 1 }
    );
    Ok(())
}

/// The commits the page offers to reattach are each package's, by name.
#[tokio::test]
async fn unattached_commits_are_listed_by_package() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    world.write("a.txt", "v2")?;
    let hash = ds
        .commit(&raw(), "second")
        .await?
        .commit
        .ok_or("nothing committed")?;
    ds.undo(&raw()).await?;

    let listed = unattached(&ds).await?;

    assert_eq!(
        listed,
        [UnattachedCommit {
            namespace: raw(),
            hash,
            message: Some("second".to_string()),
        }]
    );
    Ok(())
}

/// Keeping folders becomes the entry's folder paths in `data.toml`.
#[tokio::test]
async fn keeping_folders_becomes_folder_paths() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;

    set_scope(
        &ds,
        "team/raw",
        KeepingScope::Folders,
        vec!["data".to_string()],
    )
    .await?;

    assert_eq!(
        ds.entry(&raw()).await?.paths,
        Paths::Only(vec![PackagePath::parse("data/")?])
    );
    let refused = set_scope(&ds, "team/raw", KeepingScope::IndividualFiles, Vec::new()).await;
    assert!(refused.is_err(), "{refused:?}");
    Ok(())
}

/// A commit with nothing changed says so, as the ref path's does, and one
/// with a change is pushed by Publish.
#[tokio::test]
async fn commit_then_publish_pushes_the_commit() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;

    let nothing = commit(&ds, "team/raw", "again").await?;
    assert!(nothing.nothing_to_commit, "{nothing:?}");

    world.write("a.txt", "v2")?;
    let committed = commit(&ds, "team/raw", "second").await?;
    assert!(!committed.nothing_to_commit, "{committed:?}");
    publish(&ds, "team/raw", "Autosync").await?;

    let status = ds.entry_status(&raw(), true).await?;
    assert!(status.unpushed.is_empty(), "{status:?}");
    assert_eq!(status.hashes.latest, status.hashes.base);
    Ok(())
}

#[test]
fn a_folder_choice_names_folders_inside_the_package() {
    for folders in [
        vec![],
        vec![String::new()],
        vec!["/abs".to_string()],
        vec!["a/../b".to_string()],
    ] {
        assert!(
            scope(KeepingScope::Folders, &folders).is_err(),
            "{folders:?}"
        );
    }
    assert!(matches!(
        scope(KeepingScope::Folders, &["data/raw/".to_string()]),
        Ok(Scope::Only(paths)) if paths == ["data/raw/"]
    ));
}
