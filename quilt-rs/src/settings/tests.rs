use super::*;

use std::sync::Arc;

use test_log::test;

use crate::io::storage::mocks::MockStorage;
use quilt_uri::fixtures;

fn settings() -> (SharedSettings<Arc<MockStorage>>, Arc<MockStorage>, PathBuf) {
    let storage = Arc::new(MockStorage::default());
    let dir = storage.temp_dir.path().join(SHARED_DIR_NAME);
    (SharedSettings::new(storage.clone(), &dir), storage, dir)
}

fn host(name: &str) -> Host {
    Host::from_str(name).unwrap()
}

#[test(tokio::test)]
async fn no_file_means_no_default() -> Res {
    let (settings, _, dir) = settings();
    assert_eq!(settings.default_catalog().await?, None);
    assert!(!settings.clear_default_catalog().await?);
    // Reading and clearing create nothing.
    assert!(!dir.exists());
    Ok(())
}

#[test(tokio::test)]
async fn a_default_round_trips_as_a_bare_host() -> Res {
    let (settings, storage, dir) = settings();
    settings.set_default_catalog(&fixtures::host()).await?;
    assert_eq!(settings.default_catalog().await?, Some(fixtures::host()));

    let bytes = storage.read_bytes(dir.join(SETTINGS_FILE)).await?;
    let json: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(json, serde_json::json!({ "default_catalog": "quilt.test" }));
    assert!(dir.join(SETTINGS_LOCK).exists());
    Ok(())
}

#[test(tokio::test)]
async fn a_second_set_replaces_the_first() -> Res {
    let (settings, ..) = settings();
    settings
        .set_default_catalog(&host("one.quilt.test"))
        .await?;
    settings
        .set_default_catalog(&host("Two.Quilt.TEST"))
        .await?;
    assert_eq!(
        settings.default_catalog().await?,
        Some(host("two.quilt.test"))
    );
    Ok(())
}

#[test(tokio::test)]
async fn clearing_removes_only_the_default() -> Res {
    let (settings, storage, dir) = settings();
    storage.create_private_dir(&dir).await?;
    storage
        .write_byte_stream(
            dir.join(SETTINGS_FILE),
            br#"{"default_catalog":"quilt.test","ambient_profiles":{"b":"p"}}"#
                .to_vec()
                .into(),
        )
        .await?;

    assert!(settings.clear_default_catalog().await?);
    assert_eq!(settings.default_catalog().await?, None);
    assert!(!settings.clear_default_catalog().await?);

    let json: Value = serde_json::from_slice(&storage.read_bytes(dir.join(SETTINGS_FILE)).await?)?;
    assert_eq!(
        json,
        serde_json::json!({ "ambient_profiles": { "b": "p" } })
    );
    Ok(())
}

#[test(tokio::test)]
async fn setting_keeps_keys_it_does_not_know() -> Res {
    let (settings, storage, dir) = settings();
    storage.create_private_dir(&dir).await?;
    storage
        .write_byte_stream(
            dir.join(SETTINGS_FILE),
            br#"{"ambient_profiles":{"b":"p"}}"#.to_vec().into(),
        )
        .await?;

    settings.set_default_catalog(&fixtures::host()).await?;

    let json: Value = serde_json::from_slice(&storage.read_bytes(dir.join(SETTINGS_FILE)).await?)?;
    assert_eq!(
        json,
        serde_json::json!({
            "ambient_profiles": { "b": "p" },
            "default_catalog": "quilt.test",
        })
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_null_default_is_no_default() -> Res {
    let (settings, storage, dir) = settings();
    storage.create_private_dir(&dir).await?;
    storage
        .write_byte_stream(
            dir.join(SETTINGS_FILE),
            br#"{"default_catalog":null}"#.to_vec().into(),
        )
        .await?;
    assert_eq!(settings.default_catalog().await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_file_that_is_not_a_json_object_is_an_error_and_is_kept() -> Res {
    for content in [&b"{\"default_catalog\": "[..], b"[]"] {
        let (settings, storage, dir) = settings();
        storage.create_private_dir(&dir).await?;
        let path = dir.join(SETTINGS_FILE);
        storage
            .write_byte_stream(&path, content.to_vec().into())
            .await?;

        let err = settings.default_catalog().await.unwrap_err();
        assert!(
            err.to_string().contains(path.to_str().unwrap()),
            "{content:?}: {err}"
        );
        assert!(
            settings
                .set_default_catalog(&fixtures::host())
                .await
                .is_err()
        );
        assert!(settings.clear_default_catalog().await.is_err());
        assert_eq!(storage.read_bytes(&path).await?, content);
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_bad_default_is_an_error_to_read_and_a_set_repairs_it() -> Res {
    for content in [
        &br#"{"default_catalog": 7}"#[..],
        br#"{"default_catalog": "[::1"}"#,
    ] {
        let (settings, storage, dir) = settings();
        storage.create_private_dir(&dir).await?;
        let path = dir.join(SETTINGS_FILE);
        storage
            .write_byte_stream(&path, content.to_vec().into())
            .await?;

        let err = settings.default_catalog().await.unwrap_err();
        assert!(
            err.to_string().contains(path.to_str().unwrap()),
            "{content:?}: {err}"
        );

        settings.set_default_catalog(&fixtures::host()).await?;
        assert_eq!(settings.default_catalog().await?, Some(fixtures::host()));
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_change_waits_for_the_settings_lock() -> Res {
    let (settings, storage, dir) = settings();
    storage.create_private_dir(&dir).await?;
    let held = storage.lock_exclusive(dir.join(SETTINGS_LOCK)).await?;

    let waiting = tokio::spawn({
        let settings = settings.clone();
        async move { settings.set_default_catalog(&fixtures::host()).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!waiting.is_finished());
    assert_eq!(settings.default_catalog().await?, None);

    drop(held);
    waiting.await.unwrap()?;
    assert_eq!(settings.default_catalog().await?, Some(fixtures::host()));
    Ok(())
}

#[test]
fn the_override_names_the_folder() {
    assert_eq!(
        shared_dir_from(Some("/x/auth".into()), Some("/data".into())),
        Some(PathBuf::from("/x/auth"))
    );
}

#[test]
fn without_an_override_the_folder_is_under_the_data_dir() {
    assert_eq!(
        shared_dir_from(None, Some("/data".into())),
        Some(PathBuf::from("/data").join("com.quiltdata.auth"))
    );
    // An empty value is no value.
    assert_eq!(
        shared_dir_from(Some(PathBuf::new()), Some("/data".into())),
        Some(PathBuf::from("/data").join("com.quiltdata.auth"))
    );
    assert_eq!(shared_dir_from(None, None), None);
}

#[test]
fn the_package_catalog_beats_the_default() {
    let (one, two, default) = (
        host("one.quilt.test"),
        host("two.quilt.test"),
        host("d.quilt.test"),
    );
    // Explicit, then the package's, then (no package) the default.
    assert_eq!(
        effective_catalog(
            Some(one.clone()),
            PackageOrigin::Catalog(two.clone()),
            Some(default.clone())
        ),
        Some(one.clone())
    );
    assert_eq!(
        effective_catalog(
            None,
            PackageOrigin::Catalog(two.clone()),
            Some(default.clone())
        ),
        Some(two)
    );
    assert_eq!(
        effective_catalog(None, PackageOrigin::None, Some(default.clone())),
        Some(default.clone())
    );
    assert_eq!(effective_catalog(None, PackageOrigin::None, None), None);
    assert_eq!(
        effective_catalog(Some(one.clone()), PackageOrigin::None, Some(default)),
        Some(one)
    );
}

#[test]
fn a_package_without_a_catalog_stays_ambient() {
    let default = host("d.quilt.test");
    assert_eq!(
        effective_catalog(None, PackageOrigin::Ambient, Some(default.clone())),
        None
    );
    // Naming a catalog is still an explicit choice.
    assert_eq!(
        effective_catalog(Some(default.clone()), PackageOrigin::Ambient, None),
        Some(default)
    );
}
