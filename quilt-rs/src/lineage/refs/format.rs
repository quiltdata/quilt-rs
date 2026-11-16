//!
//! The files of per-package lineage as they are on disk, and how a ref turns
//! into a [`PackageLineage`] and back. Nothing here reads or writes a file.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Map;
use serde_json::Value;

use crate::Error;
use crate::Res;
use crate::error::LineageError;
use crate::lineage::CommitState;
use crate::lineage::Home;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::lineage::SyncScope;
use crate::logical_key::LogicalKey;
use quilt_uri::ManifestUri;

/// Top-level keys this version does not know, kept so a rewrite carries
/// them. Keys nested inside a known one are not kept, and an integer beyond
/// 64 bits comes back as a float, so a field needing either bumps `format`.
pub(crate) type Extra = Map<String, Value>;

/// What only the user and the remote can tell about one package:
/// `.quilt/refs/<owner>/<name>.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PackageRef {
    pub format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<ManifestUri>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitState>,
    #[serde(default)]
    pub sync_scope: SyncScope,
    pub tracked: BTreeSet<LogicalKey>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl PackageRef {
    pub(crate) const FORMAT: u32 = 1;

    pub(crate) fn from_slice(path: &Path, bytes: &[u8]) -> Res<Self> {
        decode(path, bytes, Self::FORMAT)
    }

    pub(crate) fn to_vec(&self) -> Res<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// The ref of `lineage`, carrying `extra` from the ref it replaces.
    /// Per-path hashes and times are not kept, nor `latest_hash`, which
    /// belongs in a [`TipCache`].
    pub(crate) fn from_lineage(lineage: &PackageLineage, extra: Extra) -> Res<Self> {
        Ok(Self {
            format: Self::FORMAT,
            remote: lineage.remote_uri.clone(),
            base_hash: lineage.base_hash.clone(),
            commit: lineage.commit.clone(),
            sync_scope: lineage.sync_scope.clone(),
            tracked: lineage
                .paths
                .keys()
                .map(|path| LogicalKey::new(path.clone()))
                .collect::<Result<_, _>>()?,
            extra,
        })
    }

    /// With no tip for the remote's bucket, `latest_hash` falls back to
    /// `base_hash` so the package does not read as behind, or to empty for
    /// a package never pushed.
    pub(crate) fn into_lineage(self, tip: Option<&TipCache>) -> PackageLineage {
        let latest_hash = match (&self.remote, tip) {
            (Some(remote), Some(tip)) if tip.bucket == remote.bucket => tip.latest_hash.clone(),
            _ => self.untipped_latest().to_string(),
        };
        PackageLineage {
            commit: self.commit,
            remote_uri: self.remote,
            base_hash: self.base_hash,
            latest_hash,
            paths: self
                .tracked
                .into_iter()
                .map(|key| (PathBuf::from(key), PathState::default()))
                .collect(),
            sync_scope: self.sync_scope,
        }
    }

    /// `latest_hash` with no tip cached for the remote's bucket.
    pub(crate) fn untipped_latest(&self) -> &str {
        match &self.remote {
            Some(remote) if !remote.hash.is_empty() => &self.base_hash,
            _ => "",
        }
    }
}

/// The domain's own state: `.quilt/domain.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct DomainConfig {
    pub format: u32,
    /// Empty when the record has none, which a writer repairs.
    #[serde(default)]
    pub home: Home,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration: Option<MigrationState>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl DomainConfig {
    pub(crate) const FORMAT: u32 = 1;

    pub(crate) fn from_slice(path: &Path, bytes: &[u8]) -> Res<Self> {
        decode(path, bytes, Self::FORMAT)
    }

    pub(crate) fn to_vec(&self) -> Res<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
}

/// Where the domain's move from `data.json` to refs stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MigrationState {
    InProgress,
    Done,
    /// Moving back to `data.json`, the refs to be archived in `.quilt/<archive>`.
    RollingBack {
        archive: String,
    },
}

/// The remote's tip as last read, beside the ref:
/// `.quilt/refs/<owner>/<name>.json.latest`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TipCache {
    pub format: u32,
    pub latest_hash: String,
    /// The bucket the tip was read from, so a cache left from another
    /// remote is not taken for this one's.
    pub bucket: String,
    pub checked_at: DateTime<Utc>,
}

impl TipCache {
    pub(crate) const FORMAT: u32 = 1;

    /// `None` when the bytes are not a cache this version reads: the tip is
    /// then unknown, never an error.
    pub(crate) fn from_slice(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice::<Self>(bytes)
            .ok()
            .filter(|tip| tip.format == Self::FORMAT)
    }

    pub(crate) fn to_vec(&self) -> Res<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
}

/// Reads the format first, so a file in another format is refused as such
/// even when the rest would not parse.
fn decode<T: DeserializeOwned>(path: &Path, bytes: &[u8], supported: u32) -> Res<T> {
    #[derive(Deserialize)]
    struct Format {
        format: u32,
    }
    let unparsed = |source| {
        Error::Lineage(LineageError::FileParse {
            path: path.to_path_buf(),
            source,
        })
    };
    let Format { format } = serde_json::from_slice(bytes).map_err(unparsed)?;
    if format != supported {
        return Err(LineageError::UnsupportedFormat {
            path: path.to_path_buf(),
            found: format,
            supported,
        }
        .into());
    }
    serde_json::from_slice(bytes).map_err(unparsed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use serde_json::json;

    const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const REMOTE: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const COMMIT: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const TIP: &str = "4444444444444444444444444444444444444444444444444444444444444444";

    fn path() -> PathBuf {
        PathBuf::from("/domain/.quilt/refs/owner/name.json")
    }

    fn read(value: &Value) -> Res<PackageRef> {
        PackageRef::from_slice(&path(), &serde_json::to_vec(value)?)
    }

    fn written(package_ref: &PackageRef) -> Res<Value> {
        Ok(serde_json::from_slice(&package_ref.to_vec()?)?)
    }

    fn keys(keys: &[&str]) -> BTreeSet<LogicalKey> {
        keys.iter()
            .map(|key| LogicalKey::new(*key).unwrap())
            .collect()
    }

    fn remote(hash: &str) -> ManifestUri {
        ManifestUri {
            origin: Some("catalog.example.com".parse().unwrap()),
            bucket: "b".to_string(),
            namespace: ("owner", "name").into(),
            hash: hash.to_string(),
        }
    }

    /// The example ref in quilt-specs' `changes/per-package-lineage-ref`.
    fn spec_example() -> Value {
        json!({
            "format": 1,
            "remote": {"origin": "catalog.example.com", "bucket": "b", "namespace": "owner/name", "hash": REMOTE},
            "base_hash": BASE,
            "commit": {"hash": COMMIT, "prev_hashes": [BASE], "timestamp": "2026-11-12T20:00:00Z"},
            "sync_scope": "entire_package",
            "tracked": ["a.csv", "data/b.csv"]
        })
    }

    fn spec_ref() -> PackageRef {
        PackageRef {
            format: 1,
            remote: Some(remote(REMOTE)),
            base_hash: BASE.to_string(),
            commit: Some(CommitState {
                timestamp: "2026-11-12T20:00:00Z".parse().unwrap(),
                hash: COMMIT.to_string(),
                prev_hashes: vec![BASE.to_string()],
            }),
            sync_scope: SyncScope::EntirePackage,
            tracked: keys(&["a.csv", "data/b.csv"]),
            extra: Extra::new(),
        }
    }

    #[test]
    fn the_spec_example_reads_and_writes_as_itself() -> Res {
        assert_eq!(read(&spec_example())?, spec_ref());
        assert_eq!(written(&spec_ref())?, spec_example());
        Ok(())
    }

    #[test]
    fn a_package_never_pushed_and_with_nothing_pending_has_no_remote_or_commit() -> Res {
        let package_ref = PackageRef {
            remote: None,
            base_hash: String::new(),
            commit: None,
            sync_scope: SyncScope::IndividualFiles,
            ..spec_ref()
        };
        let expected = json!({
            "format": 1,
            "sync_scope": "individual_files",
            "tracked": ["a.csv", "data/b.csv"]
        });
        assert_eq!(written(&package_ref)?, expected);
        assert_eq!(read(&expected)?, package_ref);
        Ok(())
    }

    #[test]
    fn a_scope_this_version_does_not_know_reads_as_individual_files() {
        let mut value = spec_example();
        value["sync_scope"] = json!({"globs": ["*.csv"]});
        let read = read(&value);
        assert!(
            matches!(
                read,
                Ok(PackageRef {
                    sync_scope: SyncScope::IndividualFiles,
                    ..
                })
            ),
            "{read:?}"
        );
    }

    #[test]
    fn a_folder_list_scope_round_trips() -> Res {
        let package_ref = PackageRef {
            sync_scope: SyncScope::folders(["data/raw", "results"])?,
            ..spec_ref()
        };
        let value = written(&package_ref)?;
        assert_eq!(
            value["sync_scope"],
            json!({"folders": ["data/raw", "results"]})
        );
        assert_eq!(read(&value)?, package_ref);
        Ok(())
    }

    #[test]
    fn unknown_top_level_keys_survive_a_rewrite() -> Res {
        let mut value = spec_example();
        value["future"] = json!({"nested": [1, 2]});
        let read = read(&value)?;
        let lineage = read.clone().into_lineage(None);
        let rewritten = PackageRef::from_lineage(&lineage, read.extra)?;
        assert_eq!(written(&rewritten)?, value);
        Ok(())
    }

    #[test]
    fn a_ref_in_another_format_is_refused_naming_the_one_supported() {
        let mut value = spec_example();
        value["format"] = json!(2);
        value["tracked"] = json!("a shape format 1 never had");
        let err = read(&value).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::Lineage(LineageError::UnsupportedFormat { path: named, found: 2, supported: 1 })
                    if *named == path()
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("reads format 1"), "{err}");
    }

    #[test]
    fn a_ref_that_does_not_parse_names_its_file() {
        for bytes in [&b"{"[..], br#"{"tracked": []}"#, br#"{"format": 1}"#] {
            let err = PackageRef::from_slice(&path(), bytes).unwrap_err();
            assert!(
                matches!(&err, Error::Lineage(LineageError::FileParse { path: named, .. }) if *named == path()),
                "{err:?}"
            );
            assert!(err.to_string().contains(&*path().to_string_lossy()));
        }
    }

    #[test]
    fn tracked_is_written_sorted_by_slash_separated_names() -> Res {
        let mut value = spec_example();
        value["tracked"] = json!(["a-b", "a/c", "a-b", "a.csv"]);
        let read = read(&value)?;
        assert_eq!(written(&read)?["tracked"], json!(["a/c", "a-b", "a.csv"]));
        Ok(())
    }

    fn lineage() -> PackageLineage {
        let state = PathState {
            timestamp: "2026-11-10T08:00:00Z".parse().unwrap(),
            ..PathState::default()
        };
        PackageLineage {
            commit: spec_ref().commit,
            remote_uri: Some(remote(REMOTE)),
            base_hash: BASE.to_string(),
            latest_hash: TIP.to_string(),
            paths: BTreeMap::from([
                (PathBuf::from("a.csv"), state.clone()),
                (PathBuf::from("data/b.csv"), state),
            ]),
            sync_scope: SyncScope::folders(["data"]).unwrap(),
        }
    }

    fn tip(bucket: &str) -> TipCache {
        TipCache {
            format: 1,
            latest_hash: TIP.to_string(),
            bucket: bucket.to_string(),
            checked_at: "2026-11-12T21:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn a_lineage_survives_the_ref_and_its_tip_but_for_per_path_state() -> Res {
        let lineage = lineage();
        let stored = PackageRef::from_lineage(&lineage, Extra::new())?;
        let stored = PackageRef::from_slice(&path(), &stored.to_vec()?)?;
        let tip = TipCache::from_slice(&tip("b").to_vec()?);
        assert_eq!(
            stored.into_lineage(tip.as_ref()),
            PackageLineage {
                paths: lineage
                    .paths
                    .keys()
                    .map(|key| (key.clone(), PathState::default()))
                    .collect(),
                ..lineage
            }
        );
        Ok(())
    }

    #[test]
    fn without_a_tip_for_its_bucket_a_package_reads_at_its_base() -> Res {
        let stored = PackageRef::from_lineage(&lineage(), Extra::new())?;
        for tip in [None, Some(tip("another-bucket"))] {
            let lineage = stored.clone().into_lineage(tip.as_ref());
            assert_eq!(lineage.latest_hash, BASE, "{tip:?}");
        }
        Ok(())
    }

    #[test]
    fn without_a_tip_a_package_never_pushed_has_no_latest() -> Res {
        let never_pushed = PackageLineage {
            remote_uri: Some(remote("")),
            ..lineage()
        };
        let stored = PackageRef::from_lineage(&never_pushed, Extra::new())?;
        assert_eq!(stored.clone().into_lineage(None).latest_hash, "");
        assert_eq!(
            stored.into_lineage(Some(&tip("b"))).latest_hash,
            TIP,
            "a tip read for it still counts"
        );
        let local = PackageLineage {
            remote_uri: None,
            ..lineage()
        };
        let stored = PackageRef::from_lineage(&local, Extra::new())?;
        assert_eq!(stored.into_lineage(Some(&tip("b"))).latest_hash, "");
        Ok(())
    }

    #[test]
    fn a_tip_cache_that_cannot_be_read_is_an_unknown_tip() -> Res {
        let mut newer = serde_json::to_value(tip("b"))?;
        newer["format"] = json!(2);
        for bytes in [
            b"".to_vec(),
            b"{".to_vec(),
            br#"{"format": 1}"#.to_vec(),
            serde_json::to_vec(&newer)?,
        ] {
            assert_eq!(TipCache::from_slice(&bytes), None);
        }
        Ok(())
    }

    #[test]
    fn a_tip_cache_holds_the_tip_its_bucket_and_when_it_was_read() -> Res {
        let expected = json!({
            "format": 1,
            "latest_hash": TIP,
            "bucket": "b",
            "checked_at": "2026-11-12T21:00:00Z"
        });
        assert_eq!(
            serde_json::from_slice::<Value>(&tip("b").to_vec()?)?,
            expected
        );
        assert_eq!(
            TipCache::from_slice(&serde_json::to_vec(&expected)?),
            Some(tip("b"))
        );
        Ok(())
    }

    fn config_path() -> PathBuf {
        PathBuf::from("/domain/.quilt/domain.json")
    }

    fn read_config(value: &Value) -> Res<DomainConfig> {
        DomainConfig::from_slice(&config_path(), &serde_json::to_vec(value)?)
    }

    #[test]
    fn a_domain_config_holds_its_home_and_migration_state() -> Res {
        for (migration, stored) in [
            (None, None),
            (Some(MigrationState::InProgress), Some(json!("in_progress"))),
            (Some(MigrationState::Done), Some(json!("done"))),
            (
                Some(MigrationState::RollingBack {
                    archive: "refs.rolled-back-x".to_string(),
                }),
                Some(json!({"rolling_back": {"archive": "refs.rolled-back-x"}})),
            ),
        ] {
            let config = DomainConfig {
                format: 1,
                home: Home::from("/home/user/QuiltSync"),
                migration,
                extra: Extra::new(),
            };
            let mut expected = json!({"format": 1, "home": "/home/user/QuiltSync"});
            if let Some(stored) = stored {
                expected["migration"] = stored;
            }
            assert_eq!(
                serde_json::from_slice::<Value>(&config.to_vec()?)?,
                expected
            );
            assert_eq!(read_config(&expected)?, config);
        }
        Ok(())
    }

    #[test]
    fn a_domain_config_without_a_home_reads_with_the_rest_kept() -> Res {
        let config = read_config(&json!({"format": 1, "migration": "done", "future": true}))?;
        assert!(config.home.as_ref().as_os_str().is_empty());
        assert_eq!(config.migration, Some(MigrationState::Done));
        assert_eq!(
            config.extra,
            Extra::from_iter([("future".into(), json!(true))])
        );
        Ok(())
    }

    #[test]
    fn a_domain_config_that_cannot_be_read_names_its_file() {
        let newer = read_config(&json!({"format": 2, "home": "/h"})).unwrap_err();
        assert!(
            matches!(
                &newer,
                Error::Lineage(LineageError::UnsupportedFormat { path, found: 2, supported: 1 })
                    if *path == config_path()
            ),
            "{newer:?}"
        );
        for value in [
            json!({"home": "/h"}),
            json!({"format": 1, "home": "/h", "migration": "paused"}),
        ] {
            let err = read_config(&value).unwrap_err();
            assert!(
                matches!(
                    &err,
                    Error::Lineage(LineageError::FileParse { path, .. }) if *path == config_path()
                ),
                "{err:?}"
            );
        }
    }
}
