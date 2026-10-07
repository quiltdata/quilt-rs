//! The observation record written last into a run's landing prefix.
//! Every sentinel is checked against the vendored schema
//! before it is written, so a malformed one never reaches S3.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;

const SCHEMA: &str = include_str!("../schemas/sentinel.schema.json");

/// The fixed key name inside `<prefix>/<instrument>/<run>/`.
pub const SENTINEL_NAME: &str = ".quilt-sentinel.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Run-relative, forward slashes.
    pub path: String,
    pub key: String,
    pub size: u64,
    pub checksum: Checksum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime_local: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sentinel {
    pub schema_version: String,
    pub sentinel_id: String,
    pub run_id: String,
    pub previous_sentinel_id: Option<String>,
    pub observer: serde_json::Value,
    pub instrument: serde_json::Value,
    pub source_path: String,
    pub boundary: serde_json::Value,
    pub instrument_local_time: Option<String>,
    pub observer_time_utc: String,
    pub snapshot_time_utc: String,
    pub checksum_algorithm: String,
    pub files: Vec<FileEntry>,
    pub file_count: usize,
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Validate a sentinel against `schemas/sentinel.schema.json`.
///
/// # Panics
///
/// Only if the schema compiled into the binary is malformed, which a test
/// in this module would already have caught.
pub fn validate(value: &serde_json::Value) -> Result<(), Error> {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).expect("vendored schema is JSON");
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .expect("vendored schema compiles");
    match validator.iter_errors(value).next() {
        None => Ok(()),
        Some(e) => Err(Error::Sentinel(format!("{e} at {}", e.instance_path()))),
    }
}

impl Sentinel {
    pub fn to_validated_bytes(&self) -> Result<Vec<u8>, Error> {
        let value = serde_json::to_value(self)?;
        validate(&value)?;
        Ok(serde_json::to_vec_pretty(&value)?)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn example() -> Sentinel {
        let raw = include_str!("../tests/fixtures/sentinel.golden.json");
        serde_json::from_str(raw).expect("golden sentinel parses")
    }

    #[test]
    fn golden_fixture_validates_and_round_trips() {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/sentinel.golden.json")).unwrap();
        validate(&raw).unwrap();
        let back = serde_json::to_value(example()).unwrap();
        assert_eq!(back, raw);
    }

    /// A timer-only boundary may never claim to be verified.
    #[test]
    fn a_guess_marked_verified_is_rejected() {
        let mut s = example();
        s.boundary["method"] = "size_stable".into();
        s.boundary["guessed"] = false.into();
        assert!(s.to_validated_bytes().is_err());
    }
}
