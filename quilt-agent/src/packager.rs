//! Turns a sealed run into a package revision (`spec/packager.md`, agent locus):
//! the sentinel's members become manifest rows pointing at the landed objects,
//! named by the RO-Crate when one exists and the profile pattern otherwise.

use std::collections::BTreeMap;
use std::path::Path;

use quilt_rs::manifest::Manifest;
use quilt_rs::manifest::ManifestHeader;
use quilt_rs::manifest::ManifestRow;
use quilt_rs::object_hash::ObjectHash;
use serde_json::Value;
use serde_json::json;

use crate::Error;
use crate::sentinel::FileEntry;
use crate::sentinel::Sentinel;

pub const CRATE_NAME: &str = "ro-crate-metadata.json";

/// An ELN anchor found in the crate. `matched` is the crate's own spelling,
/// `configured` the profile's: they differ only in case, and saying so is the
/// point (UNK-30) — a silent mismatch leaves the ELN canvas unlinked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElnAnchor {
    pub configured: String,
    pub matched: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Revision {
    pub package_name: String,
    pub manifest: Manifest,
    pub anchor: Value,
}

/// The crate's `type.id` keys: a non-file entity addressed as
/// `<lowercased @type>.<@id>`, valued by its `name` (Vir's 9/14 conventions).
fn type_id_keys(graph: &[Value]) -> Vec<(String, String)> {
    graph
        .iter()
        .filter_map(|e| {
            let t = match &e["@type"] {
                Value::String(s) => s.as_str(),
                Value::Array(a) => a.first()?.as_str()?,
                _ => return None,
            };
            if matches!(t, "File" | "Dataset" | "CreativeWork") {
                return None;
            }
            let id = e["@id"].as_str()?.trim_start_matches('#');
            let name = e["name"].as_str()?;
            (!id.is_empty()).then(|| (format!("{}.{id}", t.to_lowercase()), name.to_string()))
        })
        .collect()
}

fn find_eln(keys: &[(String, String)], eln_keys: &[String]) -> Option<ElnAnchor> {
    eln_keys.iter().find_map(|configured| {
        keys.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(configured))
            .map(|(k, v)| ElnAnchor {
                configured: configured.clone(),
                matched: k.clone(),
                value: v.clone(),
            })
    })
}

/// The revision's members: the crate's `hasPart` when present, which must be a
/// subset of what the observer landed — the crate is a claim, the sentinel the
/// observation, and the observation wins.
fn membership<'a>(
    landed: &BTreeMap<&'a str, &FileEntry>,
    dataset: Option<&'a Value>,
) -> Result<Vec<&'a str>, Error> {
    let has_part: Vec<&str> = dataset
        .and_then(|d| d["hasPart"].as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.as_str().or_else(|| p["@id"].as_str()))
                .collect()
        })
        .unwrap_or_default();
    if let Some(p) = has_part.iter().find(|p| !landed.contains_key(*p)) {
        return Err(Error::Refused(format!(
            "crate lists a file the observer did not land: {p}"
        )));
    }
    Ok(if has_part.is_empty() {
        landed.keys().copied().collect()
    } else {
        has_part
    })
}

/// One manifest row per member, pointing at the landed object version, with
/// the crate's `File` entity (minus its identity fields) as entry metadata.
fn rows(
    members: &[&str],
    landed: &BTreeMap<&str, &FileEntry>,
    graph: &[Value],
    bucket: &str,
) -> Result<Vec<ManifestRow>, Error> {
    let by_id: BTreeMap<&str, &Value> = graph
        .iter()
        .filter_map(|e| Some((e["@id"].as_str()?, e)))
        .collect();
    members
        .iter()
        .map(|path| {
            let f = landed[path];
            let hash: ObjectHash = serde_json::from_value(json!({
                "type": f.checksum.algorithm, "value": f.checksum.value
            }))
            .map_err(|e| Error::Refused(format!("{path}: unsupported checksum: {e}")))?;
            let mut physical_key = format!("s3://{bucket}/{}", f.key);
            if let Some(v) = &f.version_id {
                physical_key.push_str("?versionId=");
                physical_key.push_str(v);
            }
            let mut meta = by_id
                .get(path)
                .and_then(|e| e.as_object().cloned())
                .unwrap_or_default();
            for drop in ["@id", "@type", "name"] {
                meta.remove(drop);
            }
            Ok(ManifestRow {
                logical_key: Path::new(path).to_path_buf(),
                physical_key,
                hash,
                size: f.size,
                meta: Some(Value::Object(meta)),
            })
        })
        .collect()
}

/// Build the revision for `sentinel`. `crate_json` is the run's
/// `ro-crate-metadata.json` when it has one.
pub fn build(
    sentinel: &Sentinel,
    sentinel_key: &str,
    bucket: &str,
    folder_name: &str,
    pattern: &str,
    eln_keys: &[String],
    crate_json: Option<&Value>,
) -> Result<Revision, Error> {
    let graph: &[Value] = crate_json
        .and_then(|c| c["@graph"].as_array())
        .map_or(&[], Vec::as_slice);
    let dataset = graph
        .iter()
        .find(|e| e["@type"] == "Dataset" || e["@id"] == "./");
    let keys = type_id_keys(graph);
    let eln = find_eln(&keys, eln_keys);
    if let Some(a) = &eln
        && a.matched != a.configured
    {
        tracing::warn!(
            configured = a.configured,
            matched = a.matched,
            "ELN key matched with different case; fix the crate or the profile so the ELN links"
        );
    }

    let landed: BTreeMap<&str, &FileEntry> = sentinel
        .files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect();
    let members = membership(&landed, dataset)?;

    // Name: crate naming when it has a Namespace and a Dataset name, else the
    // profile pattern. The ELN id names nothing by itself here; it rides in
    // metadata and the ELN links the package by it.
    let prefix = graph
        .iter()
        .find(|e| e["@type"] == "Namespace")
        .and_then(|e| e["name"].as_str());
    let dataset_name = dataset.and_then(|d| d["name"].as_str());
    let package_name = match (prefix, dataset_name) {
        (Some(p), Some(n)) => format!("{p}/{n}"),
        _ => pattern
            .replace(
                "{instrument_id}",
                sentinel.instrument["id"].as_str().unwrap_or("instrument"),
            )
            .replace("{folder_name}", folder_name)
            .replace("{run_id}", &sentinel.run_id),
    };
    quilt_uri::Namespace::try_from(package_name.as_str())
        .map_err(|e| Error::Refused(format!("package name {package_name:?}: {e}")))?;

    let rows = rows(&members, &landed, graph, bucket)?;

    let mut lab_to_cloud = serde_json::to_value(sentinel)?;
    if let Some(obj) = lab_to_cloud.as_object_mut() {
        obj.remove("files");
        obj.insert("sentinel_key".into(), sentinel_key.into());
    }
    let mut user_meta = serde_json::Map::new();
    for (k, v) in &keys {
        user_meta.insert(k.clone(), v.clone().into());
    }
    user_meta.insert("lab_to_cloud".into(), lab_to_cloud);

    let anchor = match &eln {
        Some(a) => json!({
            "kind": "eln",
            "eln": { "system": "benchling", "key": a.matched, "value": a.value },
            "package_name": package_name,
        }),
        None => json!({ "kind": "on_arrival", "package_name": package_name }),
    };

    Ok(Revision {
        manifest: Manifest {
            header: ManifestHeader {
                message: Some(format!(
                    "lab-to-cloud: {} {} ({})",
                    sentinel.instrument["id"].as_str().unwrap_or(""),
                    sentinel.run_id,
                    sentinel.boundary["method"].as_str().unwrap_or("")
                )),
                user_meta: Some(Value::Object(user_meta)),
                ..ManifestHeader::default()
            },
            rows,
        },
        package_name,
        anchor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::sentinel::tests::example;

    const KEY: &str = "lab/cytoflex-lx-techops-1/20260915T101955Z-8e11c3/.quilt-sentinel.json";

    fn crate_with(eln_id: &str, has_part: &[&str]) -> Value {
        json!({ "@graph": [
            { "@id": "./", "@type": "Dataset", "name": "260908_ale", "hasPart": has_part },
            { "@id": "#ns", "@type": "Namespace", "name": "ale" },
            { "@id": eln_id, "@type": "Benchling", "name": "EXP-42" },
            { "@id": "plate3/A01.fcs", "@type": "File", "name": "A01", "dateCreated": "2026-09-08T09:14:22" }
        ]})
    }

    fn build_with(c: Option<&Value>) -> Result<Revision, Error> {
        build(
            &example(),
            KEY,
            "raw",
            "2026-09-15_ADQC_plate3",
            "{instrument_id}/{folder_name}",
            &["benchling.experiment-ID".to_string()],
            c,
        )
    }

    #[test]
    fn rows_point_at_the_landed_versions() -> Result<(), Error> {
        let rev = build_with(None)?;
        assert_eq!(
            rev.package_name,
            "cytoflex-lx-techops-1/2026-09-15_ADQC_plate3"
        );
        let row = &rev.manifest.rows[0];
        assert_eq!(
            row.physical_key,
            "s3://raw/lab/cytoflex-lx-techops-1/20260915T101955Z-8e11c3/plate3/A01.fcs?versionId=3sL4kqtJ"
        );
        assert_eq!(row.size, 4_194_304);
        let meta = rev.manifest.header.user_meta.unwrap();
        assert_eq!(meta["lab_to_cloud"]["sentinel_key"], KEY);
        assert!(meta["lab_to_cloud"].get("files").is_none());
        Ok(())
    }

    /// UNK-30: the crate spells the key `experiment-id`, the profile
    /// `experiment-ID`. Match anyway, and record the crate's spelling.
    #[test]
    fn eln_key_matches_case_insensitively_and_reports_the_spelling() -> Result<(), Error> {
        let c = crate_with("experiment-id", &["plate3/A01.fcs"]);
        let rev = build_with(Some(&c))?;
        assert_eq!(rev.anchor["kind"], "eln");
        assert_eq!(rev.anchor["eln"]["key"], "benchling.experiment-id");
        assert_eq!(rev.package_name, "ale/260908_ale");
        let row_meta = rev.manifest.rows[0].meta.as_ref().unwrap();
        assert_eq!(row_meta["dateCreated"], "2026-09-08T09:14:22");
        Ok(())
    }

    #[test]
    fn crate_claiming_an_unlanded_file_is_refused() {
        let c = crate_with("experiment-ID", &["plate3/A01.fcs", "never_landed.txt"]);
        let err = build_with(Some(&c)).unwrap_err().to_string();
        assert!(err.contains("did not land: never_landed.txt"), "{err}");
    }
}
