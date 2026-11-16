//! Manifests whose expected top hash comes from quilt3.
//!
//! Each `fixtures/quilt3-packages/<top_hash>-<case>.jsonl` is named by the top
//! hash quilt3 computes for it, and each `refused-<case>.jsonl` is a manifest
//! quilt3 refuses to load. Never rename a file to a hash quilt3 did not give.

use std::path::PathBuf;

use test_log::test;

use super::*;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::storage::LocalStorage;
use crate::io::storage::mocks::MockStorage;

fn fixture_files() -> Res<Vec<PathBuf>> {
    let dir = std::env::current_dir()?.join("fixtures/quilt3-packages");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    files.sort();
    Ok(files)
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test(tokio::test)]
async fn quilt3_fixtures_hash_to_their_names() -> Res {
    let mut checked = 0;
    for path in fixture_files()? {
        let name = file_name(&path);
        if name.starts_with("refused-") {
            continue;
        }
        let (expected, _) = name.split_once('-').expect("named <top_hash>-<case>");

        let manifest = Manifest::from_path(&LocalStorage::default(), &path).await?;
        let storage = MockStorage::default();
        let (_, top_hash) = build_manifest_from_rows_stream(
            &storage,
            storage.temp_dir.path().to_path_buf(),
            manifest.header.clone(),
            manifest.dir_meta.clone(),
            manifest.records_stream().await,
        )
        .await?;
        assert_eq!(top_hash, expected, "{name}");
        checked += 1;
    }
    assert!(checked > 0, "no quilt3 fixtures found");
    Ok(())
}

#[test(tokio::test)]
async fn quilt3_refused_fixtures_do_not_load() -> Res {
    let mut checked = 0;
    for path in fixture_files()? {
        let name = file_name(&path);
        if !name.starts_with("refused-") {
            continue;
        }
        let result = Manifest::from_path(&LocalStorage::default(), &path).await;
        assert!(result.is_err(), "{name} should not load");
        checked += 1;
    }
    assert!(checked > 0, "no refused fixtures found");
    Ok(())
}

#[test(tokio::test)]
async fn a_repeated_logical_key_is_named_in_the_error() -> Res {
    let text = concat!(
        r#"{"message":"","user_meta":{},"version":"v0"}"#,
        "\n",
        r#"{"logical_key":"data.csv","physical_keys":["s3://bucket/data.csv"],"hash":{"type":"SHA256","value":"00"},"size":1,"meta":{}}"#,
        "\n",
        r#"{"logical_key":"data.csv","physical_keys":["s3://bucket/other.csv"],"hash":{"type":"SHA256","value":"00"},"size":1,"meta":{}}"#,
        "\n",
    );
    let err = Manifest::from_reader(text.as_bytes())
        .await
        .expect_err("a repeated logical key should not load");
    assert!(err.to_string().contains("data.csv"), "{err}");
    Ok(())
}

#[test(tokio::test)]
async fn a_key_that_is_also_a_folder_is_named_in_the_error() -> Res {
    for (first, second) in [("data", "data/x.csv"), ("data/x.csv", "data")] {
        let row = |key: &str| {
            format!(
                r#"{{"logical_key":"{key}","physical_keys":["s3://bucket/{key}"],"hash":{{"type":"SHA256","value":"00"}},"size":1,"meta":{{}}}}"#
            )
        };
        let text = format!(
            "{}\n{}\n{}\n",
            r#"{"message":"","user_meta":{},"version":"v0"}"#,
            row(first),
            row(second),
        );
        let err = Manifest::from_reader(text.as_bytes())
            .await
            .expect_err("a key that is also a folder should not load");
        assert!(
            matches!(&err, Error::Manifest(ManifestError::FileAndFolder(key)) if key == "data"),
            "{first} then {second}: {err}"
        );
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_header_without_message_keeps_it_absent_on_save() -> Res {
    let path = std::env::current_dir()?.join(
        "fixtures/quilt3-packages/3426a3f721e41a1d83174c691432a39ff13720426267fc799dccf3583153e850-no-message.jsonl",
    );
    let manifest = Manifest::from_path(&LocalStorage::default(), &path).await?;

    let saved = manifest.to_jsonlines();
    let header_line = saved.lines().next().expect("a header line");
    let header: serde_json::Value = serde_json::from_str(header_line)?;
    assert!(header.get("message").is_none(), "{header_line}");

    let reread = Manifest::from_reader(saved.as_bytes()).await?;
    assert_eq!(reread, manifest);
    Ok(())
}

/// quilt3 splits keys on `/` only, so a `\\` never makes a folder, on any OS.
#[test(tokio::test)]
async fn a_backslash_is_part_of_a_name_not_a_folder() -> Res {
    let row = |key: &str| {
        format!(
            r#"{{"logical_key":{key:?},"physical_keys":["s3://bucket/x"],"hash":{{"type":"SHA256","value":"00"}},"size":1,"meta":{{}}}}"#
        )
    };
    for keys in [["data", "data\\x.csv"], ["a/b", "a\\b"]] {
        let text = format!(
            "{}\n{}\n{}\n",
            r#"{"message":"","user_meta":{},"version":"v0"}"#,
            row(keys[0]),
            row(keys[1]),
        );
        let manifest = Manifest::from_reader(text.as_bytes()).await?;
        assert_eq!(manifest.rows.len(), 2, "{keys:?}");
    }
    Ok(())
}

/// The tree check must stay linear: a deeply nested key is small on disk.
#[test(tokio::test)]
async fn a_deeply_nested_key_loads_without_copying_its_folders() -> Res {
    let key = format!("{}x", "a/".repeat(50_000));
    let text = format!(
        "{}\n{{\"logical_key\":{key:?},\"physical_keys\":[\"s3://bucket/x\"],\"hash\":{{\"type\":\"SHA256\",\"value\":\"00\"}},\"size\":1,\"meta\":{{}}}}\n",
        r#"{"message":"","user_meta":{},"version":"v0"}"#,
    );
    let started = std::time::Instant::now();
    let manifest = Manifest::from_reader(text.as_bytes()).await?;
    assert_eq!(manifest.rows.len(), 1);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}

fn json_lines(text: &str) -> Res<Vec<serde_json::Value>> {
    text.lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

/// `fixtures/quilt3-dumps/<same name>` is what quilt3 writes for the package.
#[test(tokio::test)]
async fn quilt_rs_writes_the_folder_rows_quilt3_writes() -> Res {
    let dumps = std::env::current_dir()?.join("fixtures/quilt3-dumps");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dumps)? {
        let dump = entry?.path();
        let name = file_name(&dump);
        let source = dumps.with_file_name("quilt3-packages").join(&name);
        let manifest = Manifest::from_path(&LocalStorage::default(), &source).await?;

        let mut ours = manifest.clone();
        // `records_stream` writes the files in key order; so does quilt3.
        ours.rows.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
        assert_eq!(
            json_lines(&ours.to_jsonlines())?,
            json_lines(&std::fs::read_to_string(&dump)?)?,
            "{name}"
        );
        checked += 1;
    }
    assert!(checked > 0, "no quilt3 dumps found");
    Ok(())
}

fn folder_manifest(lines: &[&str]) -> String {
    let mut text = String::from(r#"{"message":"","user_meta":{},"version":"v0"}"#);
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');
    text
}

const A_FILE: &str = r#"{"logical_key":"data/a.csv","physical_keys":["s3://bucket/a.csv"],"hash":{"type":"SHA256","value":"00"},"size":1,"meta":{}}"#;

/// quilt3 takes a row with no physical keys, an empty list or a `null` for a
/// folder's, ignores any `hash` and `size` on it, and refuses a row with no
/// `meta`, which quilt-rs reads as no metadata, like a file row without one.
#[test(tokio::test)]
async fn a_folder_row_is_a_row_without_physical_keys() -> Res {
    for row in [
        r#"{"logical_key":"data/","meta":{"a":1}}"#,
        r#"{"logical_key":"data/","physical_keys":[],"meta":{"a":1}}"#,
        r#"{"logical_key":"data/","physical_keys":null,"meta":{"a":1}}"#,
        r#"{"logical_key":"data/","hash":{"type":"SHA256","value":"00"},"size":3,"meta":{"a":1}}"#,
        r#"{"logical_key":"data/","hash":{"type":"nope"},"size":"x","meta":{"a":1}}"#,
    ] {
        let manifest = Manifest::from_reader(folder_manifest(&[row, A_FILE]).as_bytes()).await?;
        assert_eq!(manifest.rows.len(), 1, "{row}");
        assert_eq!(
            manifest.dir_meta,
            [DirMeta {
                logical_key: LogicalKey::new("data")?,
                meta: serde_json::json!({"a": 1}),
            }],
            "{row}"
        );
    }

    let manifest =
        Manifest::from_reader(folder_manifest(&[r#"{"logical_key":"data/"}"#, A_FILE]).as_bytes())
            .await?;
    assert!(!manifest.dir_meta[0].holds_meta());
    assert_eq!(manifest.rows.len(), 1);
    Ok(())
}

/// quilt3 ignores `hash` and `size` on a folder row, so a malformed hash
/// does not refuse the manifest.
#[test(tokio::test)]
async fn a_folder_row_with_a_malformed_hash_is_still_loaded() -> Res {
    let row = r#"{"logical_key":"data/","hash":{"type":"nope"},"size":"x","meta":{"a":1}}"#;
    let manifest = Manifest::from_reader(folder_manifest(&[row, A_FILE]).as_bytes()).await?;
    assert_eq!(
        manifest.dir_meta,
        [DirMeta {
            logical_key: LogicalKey::new("data")?,
            meta: serde_json::json!({"a": 1}),
        }]
    );
    assert_eq!(manifest.rows.len(), 1);
    Ok(())
}

/// A row without physical keys that is not a folder's is still refused, as
/// before folder rows were read.
#[test(tokio::test)]
async fn a_file_row_without_physical_keys_is_still_refused() -> Res {
    let row = r#"{"logical_key":"data","physical_keys":[],"hash":{"type":"SHA256","value":"00"},"size":1,"meta":{}}"#;
    let err = Manifest::from_reader(folder_manifest(&[row]).as_bytes())
        .await
        .expect_err("a file needs a physical key");
    assert_eq!(err.to_string(), "Manifest header: Physical key is missing");
    Ok(())
}

/// A folder row with an unsafe name is refused as a file's is: it would name a
/// place outside the package.
#[test(tokio::test)]
async fn a_folder_row_cannot_name_a_place_outside_the_package() -> Res {
    for key in ["../", "a/../b/", "a/ ../", "a/.. /", "./", "/", "//", "a//"] {
        let row = format!(r#"{{"logical_key":{key:?},"meta":{{"a":1}}}}"#);
        let result = Manifest::from_reader(folder_manifest(&[&row]).as_bytes()).await;
        assert!(result.is_err(), "{key:?} loaded");
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_and_a_file_of_the_same_name_are_named_in_the_error() -> Res {
    let folder = r#"{"logical_key":"data/","meta":{"a":1}}"#;
    let file = r#"{"logical_key":"data","physical_keys":["s3://bucket/data"],"hash":{"type":"SHA256","value":"00"},"size":1,"meta":{}}"#;
    for rows in [[folder, file], [file, folder]] {
        let err = Manifest::from_reader(folder_manifest(&rows).as_bytes())
            .await
            .expect_err("a name cannot be a file and a folder");
        assert!(
            matches!(&err, Error::Manifest(ManifestError::FileAndFolder(key)) if key == "data"),
            "{err}"
        );
    }
    let twice = [folder, folder];
    let err = Manifest::from_reader(folder_manifest(&twice).as_bytes())
        .await
        .expect_err("folder metadata twice");
    assert!(
        matches!(&err, Error::Manifest(ManifestError::DuplicateLogicalKey(key)) if key == "data/"),
        "{err}"
    );
    Ok(())
}

/// A folder row that says nothing is not written, but it is still a folder when
/// the manifest is checked, as it is when read.
#[test]
fn a_folder_that_says_nothing_still_clashes_with_a_file_and_with_itself() -> Res {
    let silent = || -> Res<DirMeta> {
        Ok(DirMeta {
            logical_key: LogicalKey::new("data")?,
            meta: serde_json::json!({}),
        })
    };
    let file = ManifestRow {
        logical_key: LogicalKey::new("data")?,
        ..ManifestRow::default()
    };
    let err = Manifest {
        rows: vec![file],
        dir_meta: vec![silent()?],
        ..Manifest::default()
    }
    .try_to_jsonlines()
    .expect_err("a name cannot be a file and a folder");
    assert!(
        matches!(&err, Error::Manifest(ManifestError::FileAndFolder(key)) if key == "data"),
        "{err}"
    );
    let err = Manifest {
        dir_meta: vec![silent()?, silent()?],
        ..Manifest::default()
    }
    .try_to_jsonlines()
    .expect_err("folder metadata twice");
    assert!(
        matches!(&err, Error::Manifest(ManifestError::DuplicateLogicalKey(key)) if key == "data/"),
        "{err}"
    );
    Ok(())
}

/// quilt3 writes a folder row only for metadata that says something, as Python
/// sees it.
#[test]
fn only_folder_metadata_that_says_something_is_written() -> Res {
    let says = |meta| {
        DirMeta {
            logical_key: LogicalKey::new("data").unwrap(),
            meta,
        }
        .holds_meta()
    };
    for silent in [
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!(""),
        serde_json::json!(0),
        serde_json::json!(0.0),
        serde_json::json!(false),
    ] {
        assert!(!says(silent.clone()), "{silent}");
    }
    for loud in [
        serde_json::json!({"a": null}),
        serde_json::json!([0]),
        serde_json::json!("0"),
        serde_json::json!(1),
        serde_json::json!(-0.5),
        serde_json::json!(true),
    ] {
        assert!(says(loud.clone()), "{loud}");
    }
    Ok(())
}
