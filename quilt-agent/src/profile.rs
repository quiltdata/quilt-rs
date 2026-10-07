//! The agent's one configuration file: YAML on disk, validated against the
//! vendored `schemas/agent-config.schema.json` before anything is parsed, so an
//! invalid profile refuses to start and names the field.

use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::Error;

const SCHEMA: &str = include_str!("../schemas/agent-config.schema.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub observer: Observer,
    pub registry: Registry,
    #[serde(default)]
    pub spool: Spool,
    #[serde(default)]
    pub upload: Upload,
    #[serde(default)]
    pub packager: Packager,
    pub instruments: Vec<Instrument>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Observer {
    pub id: String,
    pub placement: String,
    pub spool_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Registry {
    pub url: String,
    pub credential_ref: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Spool {
    #[serde(default = "Spool::default_max_bytes")]
    pub max_bytes: u64,
}

impl Spool {
    fn default_max_bytes() -> u64 {
        50 * 1024 * 1024 * 1024
    }
}

impl Default for Spool {
    fn default() -> Self {
        Spool {
            max_bytes: Self::default_max_bytes(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Upload {
    #[serde(default = "Upload::default_retry_base_ms")]
    pub retry_base_ms: u64,
    #[serde(default = "Upload::default_retry_max_ms")]
    pub retry_max_ms: u64,
}

impl Upload {
    fn default_retry_base_ms() -> u64 {
        1_000
    }
    fn default_retry_max_ms() -> u64 {
        300_000
    }
}

impl Default for Upload {
    fn default() -> Self {
        Upload {
            retry_base_ms: Self::default_retry_base_ms(),
            retry_max_ms: Self::default_retry_max_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Packager {
    #[serde(default = "Packager::default_eln_keys")]
    pub eln_keys: Vec<String>,
}

impl Packager {
    fn default_eln_keys() -> Vec<String> {
        vec!["benchling.experiment-ID".to_string()]
    }
}

impl Default for Packager {
    fn default() -> Self {
        Packager {
            eln_keys: Self::default_eln_keys(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Instrument {
    pub id: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub software: Option<String>,
    pub software_version: Option<String>,
    pub source: Source,
    pub landing: Landing,
    pub boundary: Boundary,
    #[serde(default)]
    pub packaging: Packaging,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    #[serde(default = "Source::default_glob")]
    pub run_folder_glob: String,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default = "Source::default_poll_s")]
    pub poll_s: u64,
    /// The directory-listing cache TTL measured on this site's mount.
    /// A new file can be invisible for this long, so no quiet window may be
    /// shorter: 60 s was measured on macOS smbfs.
    #[serde(default)]
    pub dir_cache_ttl_s: u64,
}

impl Source {
    fn default_glob() -> String {
        "*".to_string()
    }
    fn default_poll_s() -> u64 {
        30
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Landing {
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Boundary {
    pub method: Method,
    #[serde(default)]
    pub markers: Vec<String>,
    #[serde(default = "Boundary::default_true")]
    pub marker_must_be_newest: bool,
    #[serde(default = "Boundary::default_confirm")]
    pub confirm_window_s: u64,
    #[serde(default = "Boundary::default_stable")]
    pub stable_for_s: u64,
    #[serde(default = "Boundary::default_min_members")]
    pub min_members: usize,
    pub manifest_kind: Option<String>,
}

impl Boundary {
    fn default_true() -> bool {
        true
    }
    fn default_confirm() -> u64 {
        120
    }
    fn default_stable() -> u64 {
        900
    }
    fn default_min_members() -> usize {
        1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    MarkerFile,
    VendorManifest,
    Explicit,
    SizeStable,
}

impl Method {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Method::MarkerFile => "marker_file",
            Method::VendorManifest => "vendor_manifest",
            Method::Explicit => "explicit",
            Method::SizeStable => "size_stable",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Packaging {
    #[serde(default = "Packaging::default_pattern")]
    pub package_name_pattern: String,
}

impl Packaging {
    fn default_pattern() -> String {
        "{instrument_id}/{folder_name}".to_string()
    }
}

impl Default for Packaging {
    fn default() -> Self {
        Packaging {
            package_name_pattern: Self::default_pattern(),
        }
    }
}

impl Instrument {
    /// The quiet window this instrument's boundary waits out, floored by the
    /// mount's listing-cache TTL.
    #[must_use]
    pub fn quiet_window_s(&self) -> u64 {
        let configured = match self.boundary.method {
            Method::SizeStable => self.boundary.stable_for_s,
            _ => self.boundary.confirm_window_s,
        };
        configured.max(self.source.dir_cache_ttl_s)
    }
}

impl Profile {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text)
    }

    /// # Panics
    ///
    /// Only if the schema compiled into the binary is malformed, which a test
    /// in this module would already have caught.
    pub fn parse(yaml: &str) -> Result<Self, Error> {
        let value: serde_json::Value =
            serde_yaml::from_str(yaml).map_err(|e| Error::Profile(format!("not YAML: {e}")))?;
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA).expect("vendored schema is valid JSON");
        let validator = jsonschema::validator_for(&schema).expect("vendored schema compiles");
        if let Some(err) = validator.iter_errors(&value).next() {
            return Err(Error::Profile(format!(
                "{} at {}",
                err,
                err.instance_path()
            )));
        }
        serde_json::from_value(value).map_err(|e| Error::Profile(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const MINIMAL: &str = r#"
schema_version: "1"
observer: { id: edge-01, placement: beside }
registry: { url: "https://example.quiltdata.com", credential_ref: quilt-agent/x }
instruments:
  - id: plate-reader-1
    source: { path: /data/plate-reader, dir_cache_ttl_s: 60 }
    landing: { bucket: raw, prefix: lab }
    boundary: { method: marker_file, markers: ["done.txt"], confirm_window_s: 30 }
"#;

    #[test]
    fn parses_and_applies_defaults() {
        let p = Profile::parse(MINIMAL).unwrap();
        let i = &p.instruments[0];
        assert_eq!(i.boundary.method, Method::MarkerFile);
        assert!(i.boundary.marker_must_be_newest);
        assert_eq!(
            i.packaging.package_name_pattern,
            "{instrument_id}/{folder_name}"
        );
        assert_eq!(p.packager.eln_keys, ["benchling.experiment-ID"]);
    }

    #[test]
    fn ttl_floors_the_quiet_window() {
        let p = Profile::parse(MINIMAL).unwrap();
        // confirm_window_s is 30, the measured cache TTL 60: the TTL wins.
        assert_eq!(p.instruments[0].quiet_window_s(), 60);
    }

    #[test]
    fn invalid_profile_names_the_field() {
        let bad = MINIMAL.replace("marker_file, markers: [\"done.txt\"]", "marker_file");
        let err = Profile::parse(&bad).unwrap_err().to_string();
        assert!(err.contains("markers"), "{err}");
    }
}
