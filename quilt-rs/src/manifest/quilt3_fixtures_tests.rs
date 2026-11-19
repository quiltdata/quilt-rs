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

fn is_folder_line(line: &str) -> bool {
    line.contains(r#""logical_key":"#)
        && !line.contains(r#""physical_keys":"#)
        && logical_key_of(line).is_some_and(|key| key.ends_with('/'))
}

fn logical_key_of(line: &str) -> Option<&str> {
    let (_, rest) = line.split_once(r#""logical_key":"#)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
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
    let mut folder_rows = 0;
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
            if is_folder_line(&quilt3) {
                folder_rows += 1;
                assert_eq!(
                    compact(meta_text(ours)),
                    compact(meta_text(&quilt3)),
                    "{name}: folder {ours}"
                );
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
            reread.dir_meta.clone(),
            reread.records_stream().await,
        )
        .await?;
        assert_eq!(top_hash, expected_hash, "{name}: top hash after writing");
        checked += 1;
    }
    assert!(checked > 0, "no quilt3 written manifests found");
    assert!(folder_rows > 0, "no folder rows in quilt3-written fixtures");
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
        let quilt3 = std::fs::read_to_string(&dump)?;
        assert_eq!(
            json_lines(&ours.to_jsonlines())?,
            json_lines(&quilt3)?,
            "{name}"
        );
        let ours_text = ours.to_jsonlines();
        let ours_folders: Vec<_> = ours_text
            .lines()
            .filter(|line| is_folder_line(line))
            .map(|line| compact(meta_text(line).trim()))
            .collect();
        let quilt3_folders: Vec<_> = quilt3
            .lines()
            .filter(|line| is_folder_line(line))
            .map(|line| compact(meta_text(line).trim()))
            .collect();
        assert_eq!(ours_folders, quilt3_folders, "{name}: folder text");
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
            [DirMeta::new(
                LogicalKey::new("data")?,
                serde_json::json!({"a": 1}),
            )],
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
        [DirMeta::new(
            LogicalKey::new("data")?,
            serde_json::json!({"a": 1}),
        )]
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
        Ok(DirMeta::new(
            LogicalKey::new("data")?,
            serde_json::json!({}),
        ))
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

/// A manifest with folder rows still reads each file row's big integers as
/// text, and writes them back as they were.
#[test(tokio::test)]
async fn file_rows_beside_folder_rows_keep_the_text_of_big_integers() -> Res {
    let file = r#"{"logical_key":"data/a.csv","physical_keys":["s3://bucket/a.csv"],"hash":{"type":"SHA256","value":"00"},"size":1,"meta":{"n":18446744073709551617,"k":"v"}}"#;
    let folder = r#"{"logical_key":"data/","meta":{"a":1}}"#;
    let text = folder_manifest(&[folder, file]);

    let manifest = Manifest::from_reader(text.as_bytes()).await?;

    assert_eq!(manifest.dir_meta.len(), 1);
    assert!(!manifest.rows[0].meta_numbers.is_empty());
    let written = manifest.try_to_jsonlines()?;
    assert!(
        written.contains(r#""meta":{"n":18446744073709551617,"k":"v"}"#),
        "{written}"
    );
    assert_eq!(Manifest::from_reader(written.as_bytes()).await?, manifest);
    Ok(())
}

/// quilt3 writes a folder row only for metadata that says something, as Python
/// sees it.
#[test]
fn only_folder_metadata_that_says_something_is_written() -> Res {
    let says = |meta| DirMeta::new(LogicalKey::new("data").unwrap(), meta).holds_meta();
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

/// A folder row's 30-digit integer, `-0` and `1e-7` read and write as quilt3
/// writes them, and the top hash is the one without the folder row.
#[test(tokio::test)]
async fn folder_numbers_round_trip_and_leave_the_top_hash_alone() -> Res {
    let name =
        "3c7a803603e24249b0f7e0e55d35ec353c9572537f4817e0ffd525acddd0b2d0-dir-meta-numbers.jsonl";
    let (expected_hash, _) = name.split_once('-').expect("named <top_hash>-<case>");
    let source = std::env::current_dir()?
        .join("fixtures/quilt3-packages")
        .join(name);
    let manifest = Manifest::from_path(&LocalStorage::default(), &source).await?;

    assert_eq!(manifest.dir_meta.len(), 1);
    assert!(!manifest.dir_meta[0].meta_numbers.is_empty());
    assert_eq!(manifest.dir_meta[0].meta["neg0"], serde_json::json!(0));
    let written = manifest.to_jsonlines();
    assert!(
        written.contains("123456789012345678901234567890"),
        "{written}"
    );
    assert!(written.contains(r#""neg0":0"#), "{written}");
    assert!(written.contains(r#""small":1e-07"#), "{written}");
    assert!(written.contains("café"), "{written}");

    let reread = Manifest::from_reader(written.as_bytes()).await?;
    assert_eq!(reread, manifest);
    assert!(reread.dir_meta[0].same_meta(&manifest.dir_meta[0]));
    assert_eq!(reread.to_jsonlines(), written);

    let storage = MockStorage::default();
    let hash_of = async |manifest: Manifest| -> Res<String> {
        let (_, hash) = build_manifest_from_rows_stream(
            &storage,
            storage.temp_dir.path().to_path_buf(),
            manifest.header.clone(),
            manifest.dir_meta.clone(),
            manifest.records_stream().await,
        )
        .await?;
        Ok(hash)
    };
    let with = hash_of(manifest.clone()).await?;
    let mut without = manifest.clone();
    without.dir_meta.clear();
    let missing = hash_of(without).await?;
    assert_eq!(with, expected_hash);
    assert_eq!(missing, expected_hash);
    Ok(())
}
