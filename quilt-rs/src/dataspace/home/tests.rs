use super::*;

use test_log::test;

use crate::dataspace::contract::CONTRACT_FILE;
use crate::io::storage::LocalStorage;

type Res = Result<(), Box<dyn std::error::Error>>;

/// A folder that is no dataspace is made one, with an empty `data.toml`,
/// and the application's folder records it as the default.
#[test(tokio::test)]
async fn a_new_home_is_made_a_dataspace_and_recorded() -> Res {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let app = dir.path().join("app");
    let folder = dir.path().join("Quilt").join("work");

    assert_eq!(default_dataspace(&storage, &app).await?, None);
    let home = set_home(&storage, &app, &folder).await?;

    assert!(home.made);
    assert_eq!(home.dataspace, folder);
    assert_eq!(std::fs::read(folder.join(CONTRACT_FILE))?, b"");
    assert_eq!(default_dataspace(&storage, &app).await?, Some(folder));
    assert!(app.join(DEFAULT_DATASPACE_FILE).exists());
    Ok(())
}

/// A dataspace already there keeps its contract as it is, and the old
/// default keeps its own: nothing moves.
#[test(tokio::test)]
async fn choosing_another_home_moves_nothing() -> Res {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let app = dir.path().join("app");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    std::fs::create_dir_all(&second)?;
    let contract = "registry = \"s3://b\" # ours\n";
    std::fs::write(second.join(CONTRACT_FILE), contract)?;
    set_home(&storage, &app, &first).await?;
    std::fs::write(first.join("kept.txt"), b"mine")?;

    let home = set_home(&storage, &app, &second).await?;

    assert!(!home.made);
    assert_eq!(
        std::fs::read_to_string(second.join(CONTRACT_FILE))?,
        contract
    );
    assert_eq!(std::fs::read(first.join("kept.txt"))?, b"mine");
    assert!(first.join(CONTRACT_FILE).exists());
    assert_eq!(default_dataspace(&storage, &app).await?, Some(second));
    Ok(())
}

/// A `data.toml` that is not TOML is not chosen, and the default stays.
#[test(tokio::test)]
async fn a_folder_with_a_broken_contract_is_not_chosen() -> Res {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let app = dir.path().join("app");
    let first = dir.path().join("first");
    set_home(&storage, &app, &first).await?;
    let broken = dir.path().join("broken");
    std::fs::create_dir_all(&broken)?;
    std::fs::write(broken.join(CONTRACT_FILE), "registry = [")?;

    assert!(set_home(&storage, &app, &broken).await.is_err());
    assert_eq!(default_dataspace(&storage, &app).await?, Some(first));
    Ok(())
}
