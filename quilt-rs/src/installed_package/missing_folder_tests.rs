//! A package folder that is gone is an unavailable working copy, not a package
//! whose files were all deleted.
//! <https://github.com/quiltdata/quilt-rs/issues/1395>

use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::local_domain::LocalDomain;

/// A package with `a.txt` and `sub/b.txt` committed, and its working folder.
async fn committed_package(temp_dir: &TempDir) -> Res<(LocalDomain, InstalledPackage)> {
    let domain = LocalDomain::new(temp_dir.path()).with_lineage_store(BACKEND);
    domain.set_home(temp_dir.path()).await?;
    let (package, _) = domain
        .create_package(
            ("demo", "pkg").into(),
            None,
            Some("empty".to_string()),
            UserMeta::Clear,
        )
        .await?;
    let home = package.package_home().await?;
    std::fs::create_dir_all(home.join("sub"))?;
    std::fs::write(home.join("a.txt"), b"a")?;
    std::fs::write(home.join("sub/b.txt"), b"b")?;
    package
        .commit("add".to_string(), UserMeta::Keep, None, None)
        .await?;
    Ok((domain, package))
}

#[test(tokio::test)]
async fn a_missing_folder_is_unavailable_and_a_commit_refuses() -> Res {
    let temp_dir = TempDir::new()?;
    let (domain, package) = committed_package(&temp_dir).await?;
    let home = package.package_home().await?;
    let committed = package.lineage().await?.commit;
    assert!(committed.is_some());

    std::fs::remove_dir_all(&home)?;

    let status = package.status(None).await?;
    assert_eq!(status.unavailable_home(), Some(home.as_path()));
    assert!(status.changes.is_empty(), "{:?}", status.changes);

    let error = package
        .commit("after rm".to_string(), UserMeta::Keep, None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            Error::PackageOp(PackageOpError::WorkingCopyUnavailable(named)) if *named == home
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains(&home.display().to_string()));
    // Refusing leaves the folder missing rather than creating it empty.
    assert!(!home.exists());
    assert_eq!(package.lineage().await?.commit, committed);
    assert_eq!(package.manifest().await?.rows.len(), 2);

    // Nor does a download recreate it with only the files fetched.
    let error = package
        .install_paths(&[PathBuf::from("a.txt")])
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            Error::PackageOp(PackageOpError::WorkingCopyUnavailable(named)) if *named == home
        ),
        "{error:?}"
    );
    assert!(!home.exists());

    // Restored, the copy works again, with nothing changed.
    std::fs::create_dir_all(home.join("sub"))?;
    std::fs::write(home.join("a.txt"), b"a")?;
    std::fs::write(home.join("sub/b.txt"), b"b")?;
    let status = package.status(None).await?;
    assert_eq!(status.unavailable_home(), None);
    assert!(status.changes.is_empty(), "{:?}", status.changes);

    // Or uninstalled.
    std::fs::remove_dir_all(&home)?;
    domain.uninstall_package(("demo", "pkg").into()).await?;
    Ok(())
}

#[test(tokio::test)]
async fn files_deleted_from_an_existing_folder_are_still_removed() -> Res {
    let temp_dir = TempDir::new()?;
    let (_, package) = committed_package(&temp_dir).await?;
    let home = package.package_home().await?;

    std::fs::remove_file(home.join("a.txt"))?;
    std::fs::remove_dir_all(home.join("sub"))?;

    let status = package.status(None).await?;
    assert_eq!(status.unavailable_home(), None);
    let removed: Vec<_> = status
        .changes
        .iter()
        .map(|(key, change)| {
            (
                key.to_str().unwrap(),
                matches!(change, lineage::Change::Removed(_)),
            )
        })
        .collect();
    assert_eq!(removed, [("a.txt", true), ("sub/b.txt", true)]);

    package
        .commit("delete".to_string(), UserMeta::Keep, None, None)
        .await?;
    assert_eq!(package.manifest().await?.rows.len(), 0);
    Ok(())
}

/// The stat cache keeps what a walk saved through a status that could not read
/// the folder, so the restored copy is not hashed afresh.
#[test(tokio::test)]
async fn an_unavailable_walk_leaves_the_stat_cache_alone() -> Res {
    let temp_dir = TempDir::new()?;
    let store = Arc::new(PackageStateStore::in_memory());
    let domain = LocalDomain::new(temp_dir.path())
        .with_lineage_store(BACKEND)
        .with_state_store(Arc::clone(&store));
    domain.set_home(temp_dir.path()).await?;
    let namespace: Namespace = ("demo", "pkg").into();
    let (package, _) = domain
        .create_package(
            namespace.clone(),
            None,
            Some("empty".to_string()),
            UserMeta::Clear,
        )
        .await?;
    let home = package.package_home().await?;
    std::fs::write(home.join("a.txt"), b"a")?;
    // Hashed once, so the cache saves its entry.
    package
        .commit("add".to_string(), UserMeta::Keep, None, None)
        .await?;
    package.status(None).await?;
    let saved = store.stat_cache(&namespace).await.gone();
    assert_eq!(saved, [PathBuf::from("a.txt")]);

    let moved = temp_dir.path().join("moved");
    std::fs::rename(&home, &moved)?;
    assert!(package.status(None).await?.unavailable_home().is_some());
    assert_eq!(store.stat_cache(&namespace).await.gone(), saved);
    Ok(())
}

fn refuses_naming(error: &Error, home: &Path) -> bool {
    matches!(
        error,
        Error::PackageOp(PackageOpError::WorkingCopyUnavailable(named)) if named == home
    )
}

/// Writing a rule must not recreate the folder holding only `.quiltignore`:
/// every tracked file would read as removed, and autosync would publish that.
#[test(tokio::test)]
async fn a_rule_is_never_written_into_a_missing_folder() -> Res {
    let temp_dir = TempDir::new()?;
    let (_, package) = committed_package(&temp_dir).await?;
    let home = package.package_home().await?;
    std::fs::write(home.join("a.log"), b"log")?;

    std::fs::remove_dir_all(&home)?;

    let error = package
        .add_to_quiltignore(&["*.tmp".to_string()])
        .await
        .unwrap_err();
    assert!(refuses_naming(&error, &home), "{error:?}");
    // With the folder gone no rule is read, so nothing hides the file and
    // there are no lines to write: the folder stays missing.
    let lines = package
        .stop_ignoring(Path::new("a.log"), false, Scope::Package)
        .await?;
    assert_eq!(lines, Some(Vec::new()));
    assert!(!home.exists());
    assert_eq!(package.status(None).await?.unavailable_home(), Some(&*home));

    // Restored, the rule lands.
    std::fs::create_dir_all(home.join("sub"))?;
    std::fs::write(home.join("a.txt"), b"a")?;
    std::fs::write(home.join("sub/b.txt"), b"b")?;
    package.add_to_quiltignore(&["*.tmp".to_string()]).await?;
    assert_eq!(
        std::fs::read_to_string(home.join(".quiltignore"))?,
        "*.tmp\n"
    );
    Ok(())
}

/// `stop_ignoring` at [`Scope::Package`] writes into the package folder, so it
/// must refuse when that would recreate a folder holding only the exception.
#[test(tokio::test)]
async fn stop_ignoring_in_this_package_refuses_a_missing_folder() -> Res {
    let temp_dir = TempDir::new()?;
    let (_, package) = committed_package(&temp_dir).await?;
    let home = package.package_home().await?;
    std::fs::remove_dir_all(&home)?;

    let error = package
        .stop_ignoring(Path::new(".snakemake"), true, Scope::Package)
        .await
        .unwrap_err();
    assert!(refuses_naming(&error, &home), "{error:?}");
    assert!(!home.exists());
    Ok(())
}

#[test(tokio::test)]
async fn a_package_tracking_nothing_still_takes_a_rule_in_a_missing_folder() -> Res {
    let temp_dir = TempDir::new()?;
    let domain = LocalDomain::new(temp_dir.path()).with_lineage_store(BACKEND);
    domain.set_home(temp_dir.path()).await?;
    let (package, _) = domain
        .create_package(
            ("demo", "empty").into(),
            None,
            Some("empty".to_string()),
            UserMeta::Clear,
        )
        .await?;
    let home = package.package_home().await?;
    std::fs::remove_dir_all(&home)?;

    package.add_to_quiltignore(&["*.tmp".to_string()]).await?;
    assert_eq!(
        std::fs::read_to_string(home.join(".quiltignore"))?,
        "*.tmp\n"
    );
    Ok(())
}

/// A lineage record that does not parse stops a commit before it creates any
/// folder, so a missing one is not recreated empty.
#[test(tokio::test)]
async fn an_unreadable_record_does_not_recreate_a_missing_folder() -> Res {
    let temp_dir = TempDir::new()?;
    let (domain, package) = committed_package(&temp_dir).await?;
    let home = package.package_home().await?;
    std::fs::remove_dir_all(&home)?;
    domain
        .lineage_io()
        .damage_fixture(&LocalStorage::new(), &("demo", "other").into())
        .await?;

    let result = package
        .commit("after rm".to_string(), UserMeta::Keep, None, None)
        .await;

    assert!(result.is_err(), "{result:?}");
    assert!(!home.exists());
    Ok(())
}

/// What cannot be undone is said to be the folder, not "cannot overwrite".
#[test(tokio::test)]
async fn undoing_a_commit_refuses_in_a_missing_folder() -> Res {
    let temp_dir = TempDir::new()?;
    let (_, package) = committed_package(&temp_dir).await?;
    let home = package.package_home().await?;
    let committed = package.lineage().await?.commit;

    std::fs::remove_dir_all(&home)?;

    let error = package.undo_commit().await.unwrap_err();
    assert!(refuses_naming(&error, &home), "{error:?}");
    assert!(!home.exists());
    assert_eq!(package.lineage().await?.commit, committed);
    Ok(())
}
