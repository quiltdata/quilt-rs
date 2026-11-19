//!
//! `.quilt/data.json`, where builds from before refs keep every package's
//! lineage and the home in one file. This build reads its home while a
//! domain has not moved to refs; with the `testing` feature it also reads
//! and writes it whole, as such a build does.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::log;

use crate::Error;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::Storage;
#[cfg(any(test, feature = "testing"))]
use crate::io::storage::StorageExt;
use crate::lineage::Home;
use crate::lineage::PackageLineage;
use quilt_uri::Namespace;

#[cfg(any(test, feature = "testing"))]
mod data_json;
#[cfg(any(test, feature = "testing"))]
pub(crate) use data_json::DataJson;

/// It's essentially just a map of `PackageLineage`.
/// Represents the contents of `.quilt/data.json`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<Namespace, PackageLineage>,
    #[serde(default)]
    pub home: Home,
}

impl DomainLineage {
    pub fn new(home_dir: impl AsRef<Path>) -> Self {
        DomainLineage {
            packages: BTreeMap::new(),
            home: Home::from(home_dir),
        }
    }

    /// Returns a sorted vector of all namespaces in the lineage
    #[must_use]
    pub fn namespaces(&self) -> Vec<Namespace> {
        let mut namespaces: Vec<Namespace> = self.packages.keys().cloned().collect();
        namespaces.sort();
        namespaces
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Res<(Self, tempfile::TempDir)> {
        let temp_dir = tempfile::TempDir::new()?;
        Ok((DomainLineage::new(temp_dir.path()), temp_dir))
    }
}

impl AsRef<PathBuf> for DomainLineage {
    fn as_ref(&self) -> &PathBuf {
        self.home.as_ref()
    }
}

impl DomainLineage {
    pub fn from_slice(input: &[u8]) -> Res<Self> {
        parse_record(input, |lineage: &Self| &lineage.home)
    }
}

/// `data.json` read as `T`, all or part of the record, refused without a
/// home.
fn parse_record<T: serde::de::DeserializeOwned>(
    input: &[u8],
    home: impl FnOnce(&T) -> &Home,
) -> Res<T> {
    let record: T = serde_json::from_slice(input).map_err(|err| {
        log::error!("Failed to parse DomainLineage from `{input:?}`");
        Error::Lineage(LineageError::Parse(err))
    })?;
    if home(&record).as_ref().as_os_str().is_empty() {
        return Err(Error::Lineage(LineageError::MissingHome));
    }
    Ok(record)
}

/// The home in the `data.json` at `path`, `None` when there is no such file
/// or it has no home. A file that exists but can't be read, such as for
/// lack of permission, is an error.
pub(super) async fn stored_home(storage: &(impl Storage + Sync), path: &Path) -> Res<Option<Home>> {
    let Some(bytes) = super::refs::read_if_exists(storage, path).await? else {
        return Ok(None);
    };
    match parse_record(&bytes, |r: &OnlyHome| &r.home) {
        Ok(record) => Ok(Some(record.home)),
        Err(Error::Lineage(LineageError::MissingHome)) => Ok(None),
        Err(err) => Err(err),
    }
}

/// The home alone, so a package entry that does not parse does not stop its
/// read.
#[derive(Deserialize)]
struct OnlyHome {
    #[serde(default)]
    home: Home,
}

#[cfg(any(test, feature = "testing"))]
async fn home(storage: &(impl Storage + Sync), path: &Path) -> Res<Home> {
    let record = parse_record(&read_bytes(storage, path).await?, |r: &OnlyHome| &r.home)?;
    Ok(record.home)
}

#[cfg(any(test, feature = "testing"))]
async fn read_bytes(storage: &(impl Storage + Sync), path: &Path) -> Res<Vec<u8>> {
    match storage.read_bytes(path).await {
        Ok(bytes) => Ok(bytes),
        Err(_) if !storage.exists(path).await => Err(Error::Lineage(LineageError::Missing)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::lineage::SyncScope;

    #[test]
    fn test_syntax_error() {
        assert_eq!(
            DomainLineage::from_slice(b"err").unwrap_err().to_string(),
            "Failed to parse lineage file: expected value at line 1 column 1".to_string()
        );
    }

    #[test]
    fn test_wrong_key() {
        // NOTE: @fiskus I don't think this is developer friendly
        //       I'd like to remove serde(default), so this test fails
        assert!(DomainLineage::from_slice(br#"{"notkey": 123}"#).is_err());
    }

    #[test]
    fn test_wrong_value() {
        assert!(
            DomainLineage::from_slice(br#"{"packages": 123}"#)
                .unwrap_err()
                .to_string()
                .starts_with("Failed to parse lineage file: invalid type:")
        );
    }

    #[test]
    fn test_missing_working_directory() {
        assert_eq!(
            DomainLineage::from_slice(br#"{"packages":{}}"#)
                .unwrap_err()
                .to_string(),
            "Domain lineage missing Home directory".to_string()
        );
    }

    #[test]
    fn test_with_working_directory() -> Res {
        let lineage =
            DomainLineage::from_slice(br#"{"packages":{},"home":"/tmp/working_dir"}"#).unwrap();
        assert_eq!(lineage.as_ref(), &PathBuf::from("/tmp/working_dir"));
        Ok(())
    }

    /// A catalog host kept in a package's remote is read back whatever it is: one
    /// the rule for typed names would refuse, such as an IP address, must not make
    /// the whole `.quilt/data.json` unreadable.
    #[test]
    fn a_stored_catalog_host_is_read_back_as_stored() -> Res {
        for origin in ["quilt.test", "127.0.0.1", "[::1]", "xn--bcher-kva.example"] {
            let json = format!(
                r#"{{
                    "packages": {{
                        "test/stored": {{
                            "commit": null,
                            "remote": {{
                                "bucket": "bucket",
                                "namespace": "test/stored",
                                "hash": "abcdef0000000000000000000000000000000000000000000000000000000000",
                                "origin": "{origin}"
                            }},
                            "base_hash": "abcdef0000000000000000000000000000000000000000000000000000000000",
                            "latest_hash": "abcdef0000000000000000000000000000000000000000000000000000000000",
                            "paths": {{}}
                        }}
                    }},
                    "home": "/tmp/working_dir"
                }}"#
            );
            let lineage = DomainLineage::from_slice(json.as_bytes())
                .unwrap_or_else(|e| panic!("origin {origin}: {e}"));
            let again = serde_json::to_value(&lineage)?;
            assert_eq!(
                again["packages"]["test/stored"]["remote"]["origin"], origin,
                "origin {origin} does not round-trip"
            );
        }
        Ok(())
    }

    #[test]
    fn test_domain_lineage_from_temp_dir() -> Res {
        let (lineage, temp_dir) = DomainLineage::from_temp_dir()?;
        assert_eq!(lineage.as_ref(), &temp_dir.path().to_path_buf());
        assert!(lineage.packages.is_empty());
        Ok(())
    }

    #[test]
    fn test_namespaces() -> Res {
        let mut lineage = DomainLineage::new("/tmp/home");

        // Empty lineage should return empty vector
        assert_eq!(lineage.namespaces(), [] as [Namespace; 0]);

        // Add some packages
        lineage
            .packages
            .insert(Namespace::from(("foo", "bar")), PackageLineage::default());
        lineage
            .packages
            .insert(Namespace::from(("abc", "xyz")), PackageLineage::default());
        lineage.packages.insert(
            Namespace::from(("test", "package")),
            PackageLineage::default(),
        );

        // Check that namespaces are returned in sorted order
        let namespaces = lineage.namespaces();
        assert_eq!(namespaces.len(), 3);
        assert_eq!(namespaces[0], Namespace::from(("abc", "xyz")));
        assert_eq!(namespaces[1], Namespace::from(("foo", "bar")));
        assert_eq!(namespaces[2], Namespace::from(("test", "package")));

        Ok(())
    }

    /// A scope a later version adds, a word or an object, must not stop this
    /// version loading `data.json`: it reads as individual files.
    #[test]
    fn an_unknown_scope_reads_as_individual_files() {
        let json = br#"{"packages":{
            "a/word":{"commit":null,"base_hash":"","latest_hash":"","paths":{},"sync_scope":"by_pattern"},
            "a/object":{"commit":null,"base_hash":"","latest_hash":"","paths":{},"sync_scope":{"by_pattern":["*.csv"]}},
            "a/whole":{"commit":null,"base_hash":"","latest_hash":"","paths":{},"sync_scope":"entire_package"}
        },"home":"/home/directory"}"#;
        let lineage = DomainLineage::from_slice(json).unwrap();
        let scope = |name| &lineage.packages[&Namespace::from(("a", name))].sync_scope;
        assert_eq!(scope("word"), &SyncScope::IndividualFiles);
        assert_eq!(scope("object"), &SyncScope::IndividualFiles);
        assert_eq!(scope("whole"), &SyncScope::EntirePackage);
    }

    /// A record of `a/b` tracking `f.csv`, its entry ending in `hash_field`.
    fn path_entry(hash_field: &str) -> Vec<u8> {
        format!(r#"{{"packages":{{"a/b":{{"commit":null,"base_hash":"","latest_hash":"","paths":{{"f.csv":{{"timestamp":"2025-01-16T12:50:20.534Z"{hash_field}}}}}}}}},"home":"/home/directory"}}"#).into_bytes()
    }

    /// A later version stops writing `paths[].hash`; this one must still load
    /// the file.
    #[test]
    fn a_path_entry_without_a_hash_reads() {
        let lineage =
            DomainLineage::from_slice(&path_entry("")).expect("a path entry without a hash parses");
        let entry = &lineage.packages[&Namespace::from(("a", "b"))].paths[&PathBuf::from("f.csv")];
        assert_eq!(entry.hash, None);
    }

    #[test]
    fn a_path_entry_read_without_a_hash_is_written_without_one() {
        let lineage = DomainLineage::from_slice(&path_entry("")).unwrap();
        let written = serde_json::to_value(&lineage).unwrap();
        let entry = written["packages"]["a/b"]["paths"]["f.csv"]
            .as_object()
            .unwrap();
        assert!(!entry.contains_key("hash"), "{entry:?}");
    }

    #[test]
    fn a_null_path_hash_reads_as_none() {
        let lineage = DomainLineage::from_slice(&path_entry(r#","hash":null"#))
            .expect("a null path hash parses");
        let entry = &lineage.packages[&Namespace::from(("a", "b"))].paths[&PathBuf::from("f.csv")];
        assert_eq!(entry.hash, None);
    }

    /// An empty hash is malformed, not absent.
    #[test]
    fn an_empty_path_hash_fails_to_read() {
        let err = DomainLineage::from_slice(&path_entry(r#","hash":"""#)).unwrap_err();
        assert!(
            matches!(err, Error::Lineage(LineageError::Parse(_))),
            "{err:?}"
        );
    }
}
