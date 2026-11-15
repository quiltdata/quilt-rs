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
    assert_eq!(settings.default_catalog_or_adopt(&[]).await?, None);
    // Reading creates nothing.
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
async fn clearing_stores_null_and_keeps_other_keys() -> Res {
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
        serde_json::json!({ "default_catalog": null, "ambient_profiles": { "b": "p" } })
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

async fn put(storage: &MockStorage, path: &Path, content: &[u8]) -> Res {
    storage.create_dir_all(path.parent().unwrap()).await?;
    storage
        .write_byte_stream(path, content.to_vec().into())
        .await
}

fn quilt3_config(dir: &Path, name: &str) -> PathBuf {
    dir.join(name).join("config.yml")
}

const QUILT3_TEMPLATE: &str = "\
# Quilt3 configuration file
navigator_url: https://Open.Quiltdata.com:8443/b/
default_local_registry: \"file:///home/me/.local/share/Quilt/packages\"
registryUrl: https://registry.example.com
telemetry_disabled: false
";

#[test(tokio::test)]
async fn quilt3s_default_is_adopted_into_settings() -> Res {
    let (settings, storage, dir) = settings();
    let config = quilt3_config(storage.temp_dir.path(), "quilt3");
    put(&storage, &config, QUILT3_TEMPLATE.as_bytes()).await?;

    let adopted = settings
        .default_catalog_or_adopt(std::slice::from_ref(&config))
        .await?;
    assert_eq!(adopted, Some(host("open.quiltdata.com")));

    // Settings holds it now, so it no longer depends on the quilt3 file.
    storage.remove_file(&config).await?;
    assert_eq!(
        settings.default_catalog().await?,
        Some(host("open.quiltdata.com"))
    );
    let json: Value = serde_json::from_slice(&storage.read_bytes(dir.join(SETTINGS_FILE)).await?)?;
    assert_eq!(
        json,
        serde_json::json!({ "default_catalog": "open.quiltdata.com" })
    );
    Ok(())
}

#[test(tokio::test)]
async fn adopting_leaves_config_yml_as_it_was() -> Res {
    let (settings, storage, _) = settings();
    let config = quilt3_config(storage.temp_dir.path(), "quilt3");
    put(&storage, &config, QUILT3_TEMPLATE.as_bytes()).await?;

    settings
        .default_catalog_or_adopt(std::slice::from_ref(&config))
        .await?;

    assert_eq!(
        storage.read_bytes(&config).await?,
        QUILT3_TEMPLATE.as_bytes()
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_stored_default_beats_quilt3s() -> Res {
    let (settings, storage, _) = settings();
    let config = quilt3_config(storage.temp_dir.path(), "quilt3");
    put(&storage, &config, QUILT3_TEMPLATE.as_bytes()).await?;
    settings
        .set_default_catalog(&host("mine.quilt.test"))
        .await?;

    let found = settings
        .default_catalog_or_adopt(std::slice::from_ref(&config))
        .await?;

    assert_eq!(found, Some(host("mine.quilt.test")));
    assert_eq!(
        settings.default_catalog().await?,
        Some(host("mine.quilt.test"))
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_cleared_default_is_not_adopted_again() -> Res {
    let (settings, storage, _) = settings();
    let config = quilt3_config(storage.temp_dir.path(), "quilt3");
    put(&storage, &config, QUILT3_TEMPLATE.as_bytes()).await?;
    let files = [config];
    assert!(settings.default_catalog_or_adopt(&files).await?.is_some());

    assert!(settings.clear_default_catalog().await?);

    assert_eq!(settings.default_catalog_or_adopt(&files).await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn adopting_keeps_the_keys_settings_already_has() -> Res {
    let (settings, storage, dir) = settings();
    let config = quilt3_config(storage.temp_dir.path(), "quilt3");
    put(&storage, &config, QUILT3_TEMPLATE.as_bytes()).await?;
    put(
        &storage,
        &dir.join(SETTINGS_FILE),
        br#"{"ambient_profiles":{"b":"p"}}"#,
    )
    .await?;

    settings
        .default_catalog_or_adopt(std::slice::from_ref(&config))
        .await?;

    let json: Value = serde_json::from_slice(&storage.read_bytes(dir.join(SETTINGS_FILE)).await?)?;
    assert_eq!(
        json,
        serde_json::json!({
            "ambient_profiles": { "b": "p" },
            "default_catalog": "open.quiltdata.com",
        })
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_quilt3_config_with_no_catalog_is_no_default_and_no_error() -> Res {
    for content in [
        &b""[..],
        b"navigator_url:\n",
        b"navigator_url: ''\n",
        b"navigator_url: not a url\n",
        b"navigator_url: [1, 2]\n",
        b"registryUrl: https://registry.example.com\n",
        b"navigator_url: https://ok.quilt.test\n  bad: [",
        b"\xff\xfe",
    ] {
        let (settings, storage, dir) = settings();
        let config = quilt3_config(storage.temp_dir.path(), "quilt3");
        put(&storage, &config, content).await?;

        let found = settings
            .default_catalog_or_adopt(std::slice::from_ref(&config))
            .await?;

        assert_eq!(found, None, "{content:?}");
        assert!(!dir.join(SETTINGS_FILE).exists(), "{content:?}");
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_missing_or_unreadable_quilt3_config_is_no_default() -> Res {
    let (settings, storage, dir) = settings();
    let missing = quilt3_config(storage.temp_dir.path(), "none");
    assert_eq!(settings.default_catalog_or_adopt(&[missing]).await?, None);

    // A directory where the file should be cannot be read as one.
    let unreadable = quilt3_config(storage.temp_dir.path(), "dir");
    storage.create_dir_all(&unreadable).await?;
    assert_eq!(
        settings.default_catalog_or_adopt(&[unreadable]).await?,
        None
    );
    assert!(!dir.join(SETTINGS_FILE).exists());
    Ok(())
}

#[test(tokio::test)]
async fn the_first_quilt3_config_that_exists_is_the_one_read() -> Res {
    let (settings, storage, _) = settings();
    let first = quilt3_config(storage.temp_dir.path(), "a");
    let second = quilt3_config(storage.temp_dir.path(), "b");
    let missing = quilt3_config(storage.temp_dir.path(), "none");
    put(
        &storage,
        &first,
        b"navigator_url: https://first.quilt.test\n",
    )
    .await?;
    put(
        &storage,
        &second,
        b"navigator_url: https://second.quilt.test\n",
    )
    .await?;

    let found = settings
        .default_catalog_or_adopt(&[missing, first, second])
        .await?;

    assert_eq!(found, Some(host("first.quilt.test")));
    Ok(())
}

#[tokio::test]
async fn a_first_quilt3_config_without_a_catalog_is_not_skipped_for_the_next() -> Res {
    let (settings, storage, _) = settings();
    let first = quilt3_config(storage.temp_dir.path(), "a");
    let second = quilt3_config(storage.temp_dir.path(), "b");
    put(&storage, &first, b"registryUrl: https://r.quilt.test\n").await?;
    put(
        &storage,
        &second,
        b"navigator_url: https://second.quilt.test\n",
    )
    .await?;

    assert_eq!(
        settings.default_catalog_or_adopt(&[first, second]).await?,
        None
    );
    Ok(())
}

#[test]
fn quilt3_keeps_its_config_where_platformdirs_does() {
    // Absolute on every platform, so the XDG check sees what it would on a real path.
    let root = std::env::temp_dir();
    let (data, xdg) = (root.join("data"), root.join("xdg"));
    let config = |dir: &PathBuf, app: &str| dir.join(app).join("config.yml");
    let data_dir = || Some(data.clone());
    assert_eq!(
        quilt3_config_files_from(false, None, data_dir()),
        [config(&data, "Quilt")]
    );
    // An absolute XDG_DATA_HOME comes first; a relative one is ignored.
    assert_eq!(
        quilt3_config_files_from(false, Some(xdg.clone()), data_dir()),
        [config(&xdg, "Quilt"), config(&data, "Quilt")]
    );
    assert_eq!(
        quilt3_config_files_from(false, Some("xdg".into()), data_dir()),
        [config(&data, "Quilt")]
    );
    // Linux's data dir is the XDG one, so it is not listed twice.
    assert_eq!(
        quilt3_config_files_from(false, Some(data.clone()), data_dir()),
        [config(&data, "Quilt")]
    );
    // Windows nests the app under its author and ignores XDG_DATA_HOME.
    assert_eq!(
        quilt3_config_files_from(true, Some(xdg), data_dir()),
        [data.join("QuiltData").join("Quilt").join("config.yml")]
    );
    assert_eq!(
        quilt3_config_files_from(false, None, None),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn a_quilt3_catalog_is_the_host_of_its_navigator_url() {
    let host_of = |yaml: &str| catalog_in_quilt3_config(yaml.as_bytes());
    assert_eq!(
        host_of("navigator_url: https://Open.Quiltdata.com:8443/b/x"),
        Some(host("open.quiltdata.com"))
    );
    assert_eq!(
        host_of("navigator_url: http://localhost:3000"),
        Some(host("localhost"))
    );
    assert_eq!(host_of("navigator_url: open.quiltdata.com"), None);
}
