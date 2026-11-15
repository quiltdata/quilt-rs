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

/// `s` without the spaces after `:` and `,` that are outside strings, which is
/// all that sets quilt3's manifest text apart from ours.
fn compact(s: &str) -> String {
    let mut out = String::new();
    let (mut in_string, mut escaped, mut after_separator) = (false, false, false);
    for c in s.chars() {
        if in_string {
            out.push(c);
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        if after_separator && c == ' ' {
            after_separator = false;
            continue;
        }
        after_separator = matches!(c, ':' | ',');
        in_string = c == '"';
        out.push(c);
    }
    out
}

/// What a row line holds under `meta`, the last key, without the closing brace.
fn meta_text(line: &str) -> &str {
    let (_, meta) = line.split_once(r#""meta":"#).expect("a meta key");
    meta.strip_suffix('}').expect("an object")
}

/// `fixtures/quilt3-written/<same name>` is what quilt3 writes for the
/// package. Written back, the header and every row's `meta` are quilt3's own
/// bytes (but for the spaces quilt3 puts after `:` and `,`), which is how
/// integers beyond 64 bits, `-0` and floats come out, and the top hash after
/// writing and reading again is quilt3's.
#[test(tokio::test)]
async fn quilt3s_numbers_are_written_back_as_quilt3_writes_them() -> Res {
    let written = std::env::current_dir()?.join("fixtures/quilt3-written");
    let mut checked = 0;
    for entry in std::fs::read_dir(&written)? {
        let name = file_name(&entry?.path());
        let (expected_hash, _) = name.split_once('-').expect("named <top_hash>-<case>");
        let source = written.with_file_name("quilt3-packages").join(&name);
        let quilt3 = std::fs::read_to_string(written.join(&name))?;

        let manifest = Manifest::from_path(&LocalStorage::default(), &source).await?;
        let ours = manifest.to_jsonlines();

        let (mut ours_lines, mut quilt3_lines) = (ours.lines(), quilt3.lines());
        assert_eq!(
            ours_lines.next(),
            quilt3_lines.next().map(compact).as_deref(),
            "{name}: header"
        );
        for (ours, quilt3) in ours_lines.zip(quilt3_lines) {
            let quilt3 = compact(quilt3);
            // quilt3 writes `{}` for a `meta` that says nothing (`-0`, `0`,
            // `null`, ...); the top hash reads those as `{}` too.
            if meta_text(&quilt3) != "{}" {
                assert_eq!(meta_text(ours), meta_text(&quilt3), "{name}: {ours}");
            }
        }

        let reread = Manifest::from_reader(ours.as_bytes()).await?;
        assert_eq!(reread, manifest, "{name}: read back");
        assert_eq!(reread.to_jsonlines(), ours, "{name}: written again");
        let storage = MockStorage::default();
        let (_, top_hash) = build_manifest_from_rows_stream(
            &storage,
            storage.temp_dir.path().to_path_buf(),
            reread.header.clone(),
            reread.records_stream().await,
        )
        .await?;
        assert_eq!(top_hash, expected_hash, "{name}: top hash after writing");
        checked += 1;
    }
    assert!(checked > 0, "no quilt3 written manifests found");
    Ok(())
}

/// The fixtures need these to be unrepresentable by `serde_json::Value`.
#[test(tokio::test)]
async fn the_big_integer_fixture_keeps_the_text_of_each_big_integer() -> Res {
    let path = std::env::current_dir()?.join(
        "fixtures/quilt3-packages/c73246b3eecc0d9da9fb691d2e18a1dc241ed5c6cfa5433955667497ef178701-big-integers.jsonl",
    );
    let manifest = Manifest::from_path(&LocalStorage::default(), &path).await?;
    assert!(!manifest.header.user_meta_numbers.is_empty());
    assert!(manifest.rows.iter().any(|row| !row.meta_numbers.is_empty()));
    Ok(())
}
