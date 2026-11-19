use super::*;

use serde_json::json;
use test_log::test;

use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::TopHasher;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;

type Res = Result<(), Box<dyn std::error::Error>>;

fn row(key: &str, meta: Option<Value>) -> ManifestRow {
    ManifestRow {
        logical_key: LogicalKey::new(key).expect("key"),
        meta,
        ..ManifestRow::default()
    }
}

fn top_hash(manifest: &Manifest) -> Result<String, crate::Error> {
    let mut hasher = TopHasher::new();
    hasher.append_header(&manifest.header)?;
    for row in &manifest.rows {
        hasher.append(row)?;
    }
    Ok(hasher.finalize())
}

/// A revision with every kind of value the file holds, and `message`.
fn revision(message: HeaderMessage) -> Result<Manifest, Box<dyn std::error::Error>> {
    let header = ManifestHeader {
        message,
        user_meta: Some(json!({
            "nested": {"deeper": {"list": [1, "two", 3.5, false, null, {"in": "a table"}, [true]]}},
            "flags": [false, false],
            "nothing": null,
            "literal": "$$NULL",
            "floats": [0.1, -0.0, 1e300, 5e-324, 1.0],
            "ints": [i64::MIN, i64::MAX, 0],
            "dotted.key": "kept whole",
            "": "an empty key",
            "unicode": "héllo ✓\n\"quoted\"",
            "empty": {},
            "empty_list": [],
        })),
        workflow: Some(Workflow {
            config: "s3://b/.quilt/workflows/config.yml?versionId=v1".parse()?,
            id: Some(WorkflowId {
                id: "gate".to_string(),
                schemas: [(
                    "meta".to_string(),
                    "s3://b/schemas/meta.json?versionId=s1".parse()?,
                )]
                .into(),
            }),
        }),
        ..ManifestHeader::default()
    };
    Ok(Manifest {
        header,
        rows: vec![
            row(
                "a.txt",
                Some(json!({"user_meta": {"kind": "raw", "ok": false}})),
            ),
            row("data/b.csv", Some(json!({"user_meta": "a scalar"}))),
            row("data/c.csv", None),
            row("d.txt", Some(json!({"user_meta": null, "other": 1}))),
        ],
        dir_meta: vec![DirMeta::new(
            LogicalKey::new("data")?,
            json!({"user_meta": {"units": "m"}}),
        )],
    })
}

/// `manifest` written to the metadata file, then read back from it onto
/// `onto`.
fn write_back(manifest: &Manifest, onto: &mut Manifest) -> Result<(), String> {
    let file = MetadataFile::parse(Some(&render(manifest, &Paths::Whole)?))?;
    onto.header.message = file.message().expect("a file").clone();
    file.apply(onto, &Paths::Whole);
    Ok(())
}

#[test]
fn a_revision_written_and_read_back_keeps_its_top_hash() -> Res {
    let mut hashes = Vec::new();
    for message in [
        HeaderMessage::Missing,
        HeaderMessage::Null,
        HeaderMessage::Text(String::new()),
    ] {
        let original = revision(message)?;
        let mut cleared = original.clone();
        MetadataFile::parse(Some(""))?.apply(&mut cleared, &Paths::Whole);
        assert_ne!(cleared, original, "an empty file clears the metadata");

        write_back(&original, &mut cleared)?;

        assert_eq!(cleared, original);
        hashes.push(top_hash(&cleared)?);
        assert_eq!(hashes.last(), Some(&top_hash(&original)?));
    }
    assert!(
        hashes[0] != hashes[1] && hashes[1] != hashes[2] && hashes[0] != hashes[2],
        "a missing, a null and an empty message are three revisions"
    );
    Ok(())
}

#[test]
fn the_string_null_is_json_null_at_any_depth_with_no_escape() -> Res {
    let file = MetadataFile::parse(Some(
        "message = \"$NULL\"\n\
         [metadata.package]\n\
         x = \"$NULL\"\n\
         literal = \"$$NULL\"\n\
         list = [\"$NULL\", { y = \"$NULL\" }]\n",
    ))?;
    let mut manifest = Manifest::default();
    manifest.header.message = file.message().expect("a file").clone();
    file.apply(&mut manifest, &Paths::Whole);

    assert_eq!(manifest.header.message, HeaderMessage::Null);
    assert_eq!(
        manifest.header.user_meta,
        Some(json!({"x": null, "literal": "$$NULL", "list": [null, {"y": null}]}))
    );

    let string_null = Manifest {
        header: ManifestHeader {
            user_meta: Some(json!({"text": "$NULL"})),
            ..ManifestHeader::default()
        },
        ..Manifest::default()
    };
    let mut read_back = Manifest::default();
    write_back(&string_null, &mut read_back)?;
    assert_eq!(
        read_back.header.user_meta,
        Some(json!({"text": null})),
        "a JSON string \"$NULL\" cannot be written as itself"
    );
    Ok(())
}

#[test]
fn toml_values_json_has_not_are_refused() -> Res {
    for value in [
        "nan",
        "inf",
        "-inf",
        "1979-05-27",
        "07:32:00",
        "1979-05-27T07:32:00Z",
        "[1979-05-27]",
    ] {
        for text in [
            format!("[metadata.package]\nx = {value}"),
            format!("[metadata.entries.\"a.txt\"]\nx = {value}"),
        ] {
            assert!(MetadataFile::parse(Some(&text)).is_err(), "{text}");
        }
    }
    let quoted = MetadataFile::parse(Some("[metadata.package]\nx = \"1979-05-27\"\ny = 1.5"))?;
    let mut manifest = Manifest::default();
    quoted.apply(&mut manifest, &Paths::Whole);
    assert_eq!(
        manifest.header.user_meta,
        Some(json!({"x": "1979-05-27", "y": 1.5}))
    );
    Ok(())
}

/// TOML's integers are signed 64-bit, so a larger one is refused rather than
/// written as a float.
#[test]
fn an_integer_beyond_tomls_64_bits_is_refused() -> Res {
    for text in [
        r#"{"big": 18446744073709551616}"#,
        r#"{"big": 9223372036854775808}"#,
        r#"{"list": [-9223372036854775809]}"#,
    ] {
        let (value, numbers) = RawNumbers::parse(text)?;
        let mut package = Manifest::default();
        package.header.set_meta(value.clone(), numbers.clone());
        assert!(render(&package, &Paths::Whole).is_err(), "package: {text}");

        let (meta, meta_numbers) = RawNumbers::parse(&format!(r#"{{"user_meta": {text}}}"#))?;
        let entry = Manifest {
            rows: vec![ManifestRow {
                meta: Some(meta),
                meta_numbers,
                ..row("a.txt", None)
            }],
            ..Manifest::default()
        };
        assert!(render(&entry, &Paths::Whole).is_err(), "entry: {text}");
    }
    Ok(())
}

#[test]
fn a_file_that_is_not_the_metadata_file_is_refused() {
    for text in [
        "mesage = \"a typo\"",
        "[metadata.extra]\nx = 1",
        "[metadata]\nentries = 1",
        "metadata = 1",
        "version = \"v1\"",
        "message = 1",
        "[metadata.package",
    ] {
        assert!(MetadataFile::parse(Some(text)).is_err(), "{text}");
    }
}

#[test]
fn the_workflow_stamp_is_shown_and_not_taken() -> Res {
    let original = revision(HeaderMessage::Missing)?;

    let text = render(&original, &Paths::Whole)?;

    let written: Table = toml::from_str(&text)?;
    let stamp: Table = toml::from_str(
        "config = \"s3://b/.quilt/workflows/config.yml?versionId=v1\"\n\
         id = \"gate\"\n\
         schemas = { meta = \"s3://b/schemas/meta.json?versionId=s1\" }\n",
    )?;
    assert_eq!(written.get("workflow"), Some(&toml::Value::Table(stamp)));

    let edited =
        text.replace("\"gate\"", "\"forged\"") + "\n[metadata.system]\nauthor = \"someone\"\n";
    let mut manifest = original.clone();
    MetadataFile::parse(Some(&edited))?.apply(&mut manifest, &Paths::Whole);
    assert_eq!(manifest.header.workflow, original.header.workflow);
    Ok(())
}
