use quilt_rs::dataspace::install::Install;

use super::*;
use crate::engine::commands::package_entries::RuleSourceData;
use crate::engine::dataspace::tests::Res;
use crate::engine::dataspace::tests::URI;
use crate::engine::dataspace::tests::World;
use crate::engine::dataspace::tests::raw;

fn row<'a>(list: &'a EntryList, name: &str) -> &'a InstalledPackageEntryData {
    list.entries
        .iter()
        .find(|entry| entry.filename == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

/// A newer revision of `team/raw` pushed from another machine, `a.txt`
/// changed in it.
async fn pushed_elsewhere(world: &World) -> Res {
    let there = world.elsewhere()?;
    let theirs = there.open().await?;
    theirs.install(&Install::new(URI)).await?;
    // The first sync of a dataspace only reads, until it is filled.
    theirs.sync("filled").await?;
    there.write("a.txt", "theirs")?;
    theirs.sync("theirs").await?;
    Ok(())
}

/// The package page reads the entry's status, its revision and its folder:
/// a changed file, a file the package's rules leave out, and a nested
/// dataspace, which status left out (25o item 10).
#[tokio::test]
async fn the_package_page_reads_status_revision_and_folder() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    let base = ds.entry_status(&raw(), false).await?.hashes.base;
    world.write("a.txt", "v2")?;
    world.write(".quiltignore", "*.log\n")?;
    world.write("x.log", "noise")?;
    world.write("inner/data.toml", "")?;

    let page = package_page(&ds, "team/raw").await?;

    assert!(page.header.remote_locked);
    assert!(!page.header.has_local_commit);
    assert_eq!(
        page.header.uri.as_ref().map(|uri| uri.bucket.as_str()),
        Some("b")
    );
    assert_eq!(page.header.newer, base);
    assert_eq!(Some(page.context.revision.hash.clone()), base);
    assert_eq!(page.context.revision_count, 1);
    assert_eq!(page.context.bucket.as_deref(), Some("b"));
    assert_eq!(page.context.keeping.scope, KeepingScope::EntirePackage);
    assert_eq!(page.context.resolve, None);
    let FilesData::Listed(list) = &page.files else {
        panic!("the files were not listed");
    };
    assert_eq!(row(list, "a.txt").status, "modified");
    let log = row(list, "x.log").ignored_by.clone();
    assert_eq!(
        log.map(|by| by.source),
        Some(RuleSourceData::Package { line: 1 })
    );
    let inner = row(list, "inner");
    assert!(inner.folder);
    assert_eq!(
        inner.ignored_by.as_ref().map(|by| by.source.clone()),
        Some(RuleSourceData::NestedDataspace)
    );
    Ok(())
}

/// The Pull button's dry run reads what `latest` brings against the base:
/// a clean update with nothing changed here, and blocked where this folder
/// changed the same file otherwise.
#[tokio::test]
async fn the_pull_preview_compares_latest_with_the_base() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    let nothing = pull_preview(&ds, "team/raw").await?;
    assert_eq!(nothing.outcome, quilt::flow::PullOutcome::UpToDate);

    pushed_elsewhere(&world).await?;
    let clean = pull_preview(&ds, "team/raw").await?;
    assert_eq!(clean.outcome, quilt::flow::PullOutcome::CleanUpdate);
    assert_eq!(clean.changed, [PathBuf::from("a.txt")]);
    assert!(clean.latest_hash.is_some());

    world.write("a.txt", "mine")?;
    let blocked = pull_preview(&ds, "team/raw").await?;
    assert_eq!(
        blocked.outcome,
        quilt::flow::PullOutcome::Blocked {
            conflicts: vec![PathBuf::from("a.txt")]
        }
    );
    Ok(())
}

/// Download keeps the files it names: the entry's paths grow to cover
/// them, and they are read at the lock's base.
#[tokio::test]
async fn download_keeps_the_files_it_names() -> Res {
    let world = World::new()?;
    world.write("data/a.txt", "alpha")?;
    world.write("docs/b.txt", "bravo")?;
    let ds = world.published().await?;
    super::super::set_scope(
        &ds,
        "team/raw",
        crate::engine::commands::KeepingScope::Folders,
        vec!["data".to_string()],
    )
    .await?;
    std::fs::remove_file(world.folder().join("docs/b.txt"))?;

    let left_out = download(&ds, "team/raw", &["docs/b.txt".to_string()]).await?;

    assert_eq!(left_out, NotDownloaded::default());
    assert_eq!(world.text("docs/b.txt").as_deref(), Some("bravo"));
    let entry = ds.entry(&raw()).await?;
    assert!(entry.paths.covers(&LogicalKey::new("docs/b.txt")?));
    Ok(())
}

/// The commit page lists the folder's changes and starts from the
/// settings' message; a dataspace commit takes no workflow.
#[tokio::test]
async fn the_commit_page_lists_the_changes() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    world.write("a.txt", "v2")?;
    let settings = PublishSettings {
        message_template: Some("Review {namespace}".to_string()),
        ..PublishSettings::default()
    };

    let data = commit_data(&ds, &settings, "team/raw").await?;

    assert_eq!(data.message, "Review team/raw");
    assert!(data.remote_locked);
    assert_eq!(data.counts.changed, 1);
    assert!(matches!(data.workflows, CommitWorkflows::NotConfigured));
    let changed = data
        .entries
        .iter()
        .find(|entry| entry.filename == "a.txt")
        .map(|entry| entry.status.as_str());
    assert_eq!(changed, Some("modified"));
    Ok(())
}
