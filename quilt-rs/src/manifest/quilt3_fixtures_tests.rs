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
