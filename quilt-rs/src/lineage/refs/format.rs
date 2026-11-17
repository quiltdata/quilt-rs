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
use crate::lineage::WorkingCopyLocation;
use crate::logical_key::LogicalKey;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

/// Keys this version does not know, at the top level or in a working copy,
/// kept so a rewrite carries them. Keys nested deeper are not kept, and an
/// integer beyond 64 bits comes back as a float, so a field needing either
/// bumps `format`.
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
    /// Exactly one in a ref [`Self::from_slice`] reads at [`Self::FORMAT`],
    /// and none or one at [`Self::FORMAT_NO_COPY`].
    pub copies: Vec<RefCopy>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A folder holding the package's files.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct RefCopy {
    #[serde(default)]
    pub sync_scope: SyncScope,
    pub tracked: BTreeSet<LogicalKey>,
    /// Absent for the package's folder under the home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// Absent when the copy is at the package's `base_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl PackageRef {
    pub(crate) const FORMAT: u32 = 1;

    /// A ref with no working copy. Only such a ref is written in it, so a
    /// build that reads only [`Self::FORMAT`] refuses that package alone,
    /// and every other ref stays readable by it.
    pub(crate) const FORMAT_NO_COPY: u32 = 2;

    /// Refuses, as [`LineageError::UnsupportedCopies`], a ref whose copies
    /// this version cannot keep, so none is misread.
    pub(crate) fn from_slice(path: &Path, bytes: &[u8]) -> Res<Self> {
        let package_ref: Self = decode(path, bytes, &[Self::FORMAT, Self::FORMAT_NO_COPY])?;
        let head = &package_ref.base_hash;
        let found = match package_ref.copies.as_slice() {
            [copy] if copy.folder.is_some() => "a working copy in another folder",
            [copy] if copy.base.as_ref().is_some_and(|base| base != head) => {
                "a working copy at another base"
            }
            [_] => return Ok(package_ref),
            [] if package_ref.format == Self::FORMAT_NO_COPY => return Ok(package_ref),
            [] => "no working copy",
            _ => "more than one working copy",
        };
        Err(LineageError::UnsupportedCopies {
            path: path.to_path_buf(),
            found,
        }
        .into())
    }

    pub(crate) fn to_vec(&self) -> Res<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// The ref of `lineage`, carrying the unknown keys of the ref it
    /// replaces. Per-path hashes and times are not kept, nor `latest_hash`,
    /// which belongs in a [`TipCache`]. A package with no working copy has
    /// no copy, in [`Self::FORMAT_NO_COPY`]; any other is in [`Self::FORMAT`].
    pub(crate) fn from_lineage(lineage: &PackageLineage, replaces: Option<&Self>) -> Res<Self> {
        let copy = RefCopy {
            sync_scope: lineage.sync_scope.clone(),
            tracked: lineage
                .paths
                .keys()
                .map(|path| LogicalKey::new(path.clone()))
                .collect::<Result<_, _>>()?,
            extra: replaces
                .and_then(|replaced| replaced.copies.first())
                .map_or_default(|copy| copy.extra.clone()),
            ..RefCopy::default()
        };
        let (format, copies) = match lineage.working_copy {
            WorkingCopyLocation::Home => (Self::FORMAT, vec![copy]),
            WorkingCopyLocation::None => (Self::FORMAT_NO_COPY, Vec::new()),
        };
        Ok(Self {
            format,
            remote: lineage.remote_uri.clone(),
            base_hash: lineage.base_hash.clone(),
            commit: lineage.commit.clone(),
            copies,
            extra: replaces.map_or_default(|replaced| replaced.extra.clone()),
        })
    }

    /// `latest_hash` is the tip cached for this ref's remote package, or
    /// [`Self::untipped_latest`] when none is.
    pub(crate) fn into_lineage(self, tip: Option<&TipCache>) -> PackageLineage {
        let latest_hash = match (&self.remote, tip) {
            (Some(remote), Some(tip)) if tip.is_for(remote) => tip.latest_hash.clone(),
            _ => self.untipped_latest().to_string(),
        };
        let working_copy = if self.copies.is_empty() {
            WorkingCopyLocation::None
        } else {
            WorkingCopyLocation::Home
        };
        let copy = self.copies.into_iter().next().unwrap_or_default();
        PackageLineage {
            commit: self.commit,
            remote_uri: self.remote,
            base_hash: self.base_hash,
            latest_hash,
            paths: copy
                .tracked
                .into_iter()
                .map(|key| (PathBuf::from(key), PathState::default()))
                .collect(),
            sync_scope: copy.sync_scope,
            working_copy,
        }
    }

    /// The `latest_hash` this ref reads with its tip unknown: `base_hash`, so
    /// the package does not read as behind, or empty for a package never
    /// pushed.
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
        decode(path, bytes, &[Self::FORMAT])
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
    /// The remote package the tip was read from, so a cache left from
    /// another is not taken for this one's. Not its catalog, which only
    /// signs the read.
    pub bucket: String,
    pub namespace: Namespace,
    pub checked_at: DateTime<Utc>,
}

impl TipCache {
    pub(crate) const FORMAT: u32 = 1;

    /// `latest_hash` as read now from `remote`'s package.
    pub(crate) fn of(remote: &ManifestUri, latest_hash: &str) -> Self {
        Self {
            format: Self::FORMAT,
            latest_hash: latest_hash.to_string(),
            bucket: remote.bucket.clone(),
            namespace: remote.namespace.clone(),
            checked_at: Utc::now(),
        }
    }

    /// Whether this is the tip of `remote`'s package, at whatever revision.
    pub(crate) fn is_for(&self, remote: &ManifestUri) -> bool {
        self.bucket == remote.bucket && self.namespace == remote.namespace
    }

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
fn decode<T: DeserializeOwned>(path: &Path, bytes: &[u8], supported: &[u32]) -> Res<T> {
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
    if !supported.contains(&format) {
        return Err(LineageError::UnsupportedFormat {
            path: path.to_path_buf(),
            found: format,
            supported: supported.iter().copied().max().unwrap_or_default(),
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

    /// A pushed package with a commit pending and its one working copy.
    fn example() -> Value {
        json!({
            "format": 1,
            "remote": {"origin": "catalog.example.com", "bucket": "b", "namespace": "owner/name", "hash": REMOTE},
            "base_hash": BASE,
            "commit": {"hash": COMMIT, "prev_hashes": [BASE], "timestamp": "2026-11-12T20:00:00Z"},
            "copies": [
                {"sync_scope": "entire_package", "tracked": ["a.csv", "data/b.csv"]}
            ]
        })
    }

    fn copy(sync_scope: SyncScope) -> RefCopy {
        RefCopy {
            sync_scope,
            tracked: keys(&["a.csv", "data/b.csv"]),
            ..RefCopy::default()
        }
    }

    fn example_ref() -> PackageRef {
        PackageRef {
            format: 1,
            remote: Some(remote(REMOTE)),
            base_hash: BASE.to_string(),
            commit: Some(CommitState {
                timestamp: "2026-11-12T20:00:00Z".parse().unwrap(),
                hash: COMMIT.to_string(),
                prev_hashes: vec![BASE.to_string()],
            }),
            copies: vec![copy(SyncScope::EntirePackage)],
            extra: Extra::new(),
        }
    }

    #[test]
    fn the_example_reads_and_writes_as_itself() -> Res {
        assert_eq!(read(&example())?, example_ref());
        assert_eq!(written(&example_ref())?, example());
        Ok(())
    }

    #[test]
    fn a_package_never_pushed_and_with_nothing_pending_has_no_remote_or_commit() -> Res {
        let package_ref = PackageRef {
            remote: None,
            base_hash: String::new(),
            commit: None,
            copies: vec![copy(SyncScope::IndividualFiles)],
            ..example_ref()
        };
        let expected = json!({
            "format": 1,
            "copies": [
                {"sync_scope": "individual_files", "tracked": ["a.csv", "data/b.csv"]}
            ]
        });
        assert_eq!(written(&package_ref)?, expected);
        assert_eq!(read(&expected)?, package_ref);
        Ok(())
    }

    #[test]
    fn a_scope_this_version_does_not_know_reads_as_individual_files() -> Res {
        let mut value = example();
        value["copies"][0]["sync_scope"] = json!({"globs": ["*.csv"]});
        assert_eq!(read(&value)?.copies, [copy(SyncScope::IndividualFiles)]);
        Ok(())
    }

    #[test]
    fn a_folder_list_scope_round_trips() -> Res {
        let package_ref = PackageRef {
            copies: vec![copy(SyncScope::folders(["data/raw", "results"])?)],
            ..example_ref()
        };
        let value = written(&package_ref)?;
        assert_eq!(
            value["copies"][0]["sync_scope"],
            json!({"folders": ["data/raw", "results"]})
        );
        assert_eq!(read(&value)?, package_ref);
        Ok(())
    }

    #[test]
    fn unknown_keys_survive_a_rewrite_at_the_top_level_and_in_the_copy() -> Res {
        let mut value = example();
        value["future"] = json!({"nested": [1, 2]});
        value["copies"][0]["future_copy"] = json!(true);
        let read = read(&value)?;
        let lineage = read.clone().into_lineage(None);
        let rewritten = PackageRef::from_lineage(&lineage, Some(&read))?;
        assert_eq!(written(&rewritten)?, value);
        Ok(())
    }

    #[test]
    fn copies_this_version_cannot_keep_are_refused_naming_the_file() {
        let the_copy = example()["copies"][0].clone();
        let with = |key: &str, value: Value| {
            let mut copy = the_copy.clone();
            copy[key] = value;
            json!([copy])
        };
        let cases = [
            ("no copy", json!([])),
            ("two copies", json!([the_copy, the_copy])),
            ("a folder", with("folder", json!("/elsewhere/name"))),
            ("another base", with("base", json!(REMOTE))),
        ];
        for (case, copies) in cases {
            let mut value = example();
            value["copies"] = copies;
            let read = read(&value);
            assert!(
                matches!(
                    &read,
                    Err(Error::Lineage(LineageError::UnsupportedCopies { path: named, .. }))
                        if *named == path()
                ),
                "{case}: {read:?}"
            );
            let message = read.unwrap_err().to_string();
            assert!(message.contains(&*path().to_string_lossy()), "{message}");
        }
    }

    /// Format 2 holds a package with no working copy, and only that: a
    /// package with one is written in format 1, which every build reads.
    #[test]
    fn a_package_with_no_working_copy_has_no_copy_in_format_2() -> Res {
        let none = PackageLineage {
            working_copy: WorkingCopyLocation::None,
            ..lineage()
        };
        let package_ref = PackageRef::from_lineage(&none, Some(&example_ref()))?;
        let value = written(&package_ref)?;
        assert_eq!(value["format"], PackageRef::FORMAT_NO_COPY);
        assert_eq!(value["copies"], json!([]));
        let read_back = read(&value)?.into_lineage(None);
        assert_eq!(read_back.working_copy, WorkingCopyLocation::None);
        assert!(read_back.paths.is_empty());

        let opened = PackageLineage {
            working_copy: WorkingCopyLocation::Home,
            ..read_back
        };
        let reopened = PackageRef::from_lineage(&opened, Some(&package_ref))?;
        assert_eq!(reopened.format, PackageRef::FORMAT);
        assert_eq!(reopened.copies.len(), 1);
        Ok(())
    }

    #[test]
    fn a_copy_at_the_package_base_is_read() {
        let mut value = example();
        value["copies"][0]["base"] = json!(BASE);
        let read = read(&value);
        assert!(read.is_ok(), "{read:?}");
    }

    #[test]
    fn a_ref_tracking_at_its_top_level_is_refused_not_read_as_tracking_nothing() {
        let mut value = example();
        let copies = value.as_object_mut().unwrap().remove("copies").unwrap();
        value["sync_scope"] = copies[0]["sync_scope"].clone();
        value["tracked"] = copies[0]["tracked"].clone();
        let read = read(&value);
        assert!(
            matches!(
                &read,
                Err(Error::Lineage(LineageError::FileParse { path: named, .. })) if *named == path()
            ),
            "{read:?}"
        );
    }

    #[test]
    fn a_ref_in_another_format_is_refused_naming_the_newest_supported() {
        let mut value = example();
        value["format"] = json!(3);
        value["copies"] = json!("a shape no supported format has");
        let err = read(&value).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::Lineage(LineageError::UnsupportedFormat { path: named, found: 3, supported: 2 })
                    if *named == path()
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("reads format 2"), "{err}");
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
        let mut value = example();
        value["copies"][0]["tracked"] = json!(["a-b", "a/c", "a-b", "a.csv"]);
        let read = read(&value)?;
        assert_eq!(
            written(&read)?["copies"][0]["tracked"],
            json!(["a/c", "a-b", "a.csv"])
        );
        Ok(())
    }

    fn lineage() -> PackageLineage {
        let state = PathState {
            timestamp: "2026-11-10T08:00:00Z".parse().unwrap(),
            ..PathState::default()
        };
        PackageLineage {
            commit: example_ref().commit,
            remote_uri: Some(remote(REMOTE)),
            base_hash: BASE.to_string(),
            latest_hash: TIP.to_string(),
            paths: BTreeMap::from([
                (PathBuf::from("a.csv"), state.clone()),
                (PathBuf::from("data/b.csv"), state),
            ]),
            sync_scope: SyncScope::folders(["data"]).unwrap(),
            ..PackageLineage::default()
        }
    }

    /// `TIP`, read from the example's remote package.
    fn tip() -> TipCache {
        TipCache {
            checked_at: "2026-11-12T21:00:00Z".parse().unwrap(),
            ..TipCache::of(&remote(REMOTE), TIP)
        }
    }

    #[test]
    fn a_lineage_survives_the_ref_and_its_tip_but_for_per_path_state() -> Res {
        let lineage = lineage();
        let stored = PackageRef::from_lineage(&lineage, None)?;
        let stored = PackageRef::from_slice(&path(), &stored.to_vec()?)?;
        let tip = TipCache::from_slice(&tip().to_vec()?);
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
    fn without_a_tip_for_its_package_a_package_reads_at_its_base() -> Res {
        let stored = PackageRef::from_lineage(&lineage(), None)?;
        let other_tips = [
            TipCache {
                bucket: "another-bucket".to_string(),
                ..tip()
            },
            TipCache {
                namespace: ("owner", "another-name").into(),
                ..tip()
            },
        ];
        for tip in [None].into_iter().chain(other_tips.iter().map(Some)) {
            let lineage = stored.clone().into_lineage(tip);
            assert_eq!(lineage.latest_hash, BASE, "{tip:?}");
        }
        Ok(())
    }

    #[test]
    fn a_tip_read_through_another_catalog_is_still_the_packages() -> Res {
        let stored = PackageRef::from_lineage(&lineage(), None)?;
        let uncatalogued = ManifestUri {
            origin: None,
            ..remote(REMOTE)
        };
        let tip = TipCache::of(&uncatalogued, TIP);
        assert_eq!(stored.into_lineage(Some(&tip)).latest_hash, TIP);
        Ok(())
    }

    #[test]
    fn without_a_tip_a_package_never_pushed_has_no_latest() -> Res {
        let never_pushed = PackageLineage {
            remote_uri: Some(remote("")),
            ..lineage()
        };
        let stored = PackageRef::from_lineage(&never_pushed, None)?;
        assert_eq!(stored.clone().into_lineage(None).latest_hash, "");
        assert_eq!(
            stored.into_lineage(Some(&tip())).latest_hash,
            TIP,
            "a tip read for it still counts"
        );
        let local = PackageLineage {
            remote_uri: None,
            ..lineage()
        };
        let stored = PackageRef::from_lineage(&local, None)?;
        assert_eq!(stored.into_lineage(Some(&tip())).latest_hash, "");
        Ok(())
    }

    #[test]
    fn a_tip_cache_that_cannot_be_read_is_an_unknown_tip() -> Res {
        let mut newer = serde_json::to_value(tip())?;
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
    fn a_tip_cache_holds_the_tip_its_package_and_when_it_was_read() -> Res {
        let expected = json!({
            "format": 1,
            "latest_hash": TIP,
            "bucket": "b",
            "namespace": "owner/name",
            "checked_at": "2026-11-12T21:00:00Z"
        });
        assert_eq!(serde_json::from_slice::<Value>(&tip().to_vec()?)?, expected);
        assert_eq!(
            TipCache::from_slice(&serde_json::to_vec(&expected)?),
            Some(tip())
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
