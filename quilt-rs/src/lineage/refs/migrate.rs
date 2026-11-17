//!
//! Moves a domain's lineage from `data.json` to refs in steps a run killed
//! at any point resumes from: freeze `data.json` and put the stub in its
//! place, write each package's ref, then mark the move done.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "nothing outside tests runs the move yet")
)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::RefStore;
use super::format::DomainConfig;
use super::format::MigrationState;
use super::format::PackageRef;
use super::state;
use super::state::DomainState;
use super::store::put;
use super::store::read_config;
use super::store::read_if_exists;
use super::store::remove_if_present;
use crate::Error;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::ByteStream;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::lineage::DomainLineage;
use crate::lineage::Home;
use crate::lineage::PackageLineage;
use crate::package_lock::PackageLock;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// What one run of [`up`] did.
#[derive(Debug, Default)]
pub(crate) struct Migrated {
    /// The packages whose ref this run wrote.
    pub written: BTreeSet<Namespace>,
    /// The entries left out, by their name in `data.json`.
    pub skipped: BTreeMap<String, Error>,
}

/// The keys of a `data.json` entry that [`PackageLineage`] reads. The ref
/// carries the others.
const ENTRY_KEYS: [&str; 6] = [
    "commit",
    "remote",
    "base_hash",
    "latest_hash",
    "paths",
    "sync_scope",
];

/// A frozen copy of `data.json`. Its home may be empty, as a domain with no
/// `data.json` freezes one, and its entries stay raw, so one that does not
/// parse costs only its package.
#[derive(Deserialize)]
struct Frozen {
    #[serde(default)]
    home: Home,
    #[serde(default)]
    packages: BTreeMap<String, Value>,
}

impl Frozen {
    fn from_slice(path: &Path, bytes: &[u8]) -> Res<Self> {
        serde_json::from_slice(bytes).map_err(|source| {
            LineageError::MigrationRefused {
                path: path.to_path_buf(),
                source,
            }
            .into()
        })
    }

    async fn read(storage: &(impl Storage + Sync), path: &Path) -> Res<Self> {
        Self::from_slice(path, &storage.read_bytes(path).await?)
    }
}

/// Moves the domain at `paths` to refs, or finishes a move a killed run
/// began. One process moves a domain at a time, under `migrate.lock`; one
/// that waited for it reads what the one before it left. A domain already on
/// refs is left as it is, and one being rolled back is refused.
pub(crate) async fn up(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res<Migrated> {
    let _moving = storage.lock_exclusive(paths.migrate_lock()).await?;
    let (source, frozen) = match state::detect(storage, paths).await? {
        DomainState::Refs => {
            remove_working_copy(storage, paths).await?;
            return Ok(Migrated::default());
        }
        DomainState::RollingBack { .. } => {
            return Err(LineageError::RollingBack {
                path: paths.domain_config(),
            }
            .into());
        }
        DomainState::Legacy => freeze(storage, paths).await?,
        DomainState::StubOnly { source } => {
            let frozen = Frozen::read(storage, &source).await?;
            begin(storage, paths, &frozen.home).await?;
            (source, frozen)
        }
        DomainState::Migrating { source } => {
            let frozen = Frozen::read(storage, &source).await?;
            (source, frozen)
        }
    };
    let migrated = write_refs(storage, paths, &source, frozen.packages).await?;
    crate::failpoint::hit("migrate.up.before-done");
    finish(storage, paths).await?;
    Ok(migrated)
}

/// Under `data.json.lock`, which older builds write `data.json` under: copies
/// `data.json`, or an empty lineage where there is none, to the frozen copy
/// it returns, puts the stub in its place, and records the move as begun.
/// Older builds then fail on the stub, so the copy stays as it was frozen.
async fn freeze(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res<(PathBuf, Frozen)> {
    let lineage = paths.lineage();
    let _short = storage.lock_exclusive(paths.lineage_lock()).await?;
    let bytes = match read_if_exists(storage, &lineage).await? {
        Some(bytes) => bytes,
        None => serde_json::to_vec(&DomainLineage::default())?,
    };
    let frozen = Frozen::from_slice(&lineage, &bytes)?;
    set_refs_aside(storage, paths).await?;
    // The first move's copy is kept for good.
    let source = if storage.exists(paths.lineage_pre_refs()).await {
        paths.lineage_migrating()
    } else {
        paths.lineage_pre_refs()
    };
    storage
        .write_byte_stream(&source, ByteStream::from(bytes))
        .await?;
    crate::failpoint::hit("migrate.up.after-pre-refs");
    storage
        .write_byte_stream(&lineage, ByteStream::from_static(state::STUB))
        .await?;
    crate::failpoint::hit("migrate.up.after-stub");
    begin(storage, paths, &frozen.home).await?;
    Ok((source, frozen))
}

/// Moves aside a `refs/` found before the move began. No ref is written
/// before the domain config exists, so none there is this domain's.
async fn set_refs_aside(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res {
    let refs = paths.refs_dir();
    if !storage.exists(&refs).await {
        return Ok(());
    }
    let aside = paths.refs_archive(&format!("refs.set-aside-{}", Uuid::new_v4()));
    storage.rename(&refs, &aside).await?;
    storage.sync_dir(paths.dot_quilt_dir()).await;
    Ok(())
}

async fn begin(storage: &(impl Storage + Sync), paths: &DomainPaths, home: &Home) -> Res {
    let config = DomainConfig {
        format: DomainConfig::FORMAT,
        home: home.clone(),
        migration: Some(MigrationState::InProgress),
        extra: read_config(storage, paths)
            .await?
            .map_or_default(|config| config.extra),
    };
    put(storage, &paths.domain_config(), config.to_vec()?, None).await?;
    crate::failpoint::hit("migrate.up.after-config");
    Ok(())
}

/// Writes the ref of each of `packages`, the entries frozen in `source`,
/// under its lock, unless it has one: a ref already there was written from
/// this same copy by a run that was killed.
async fn write_refs(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    source: &Path,
    packages: BTreeMap<String, Value>,
) -> Res<Migrated> {
    let store = RefStore::new(paths.clone());
    let mut migrated = Migrated::default();
    for (name, entry) in packages {
        let (namespace, new_ref, lineage) = match moved(paths, &name, entry) {
            Ok(moved) => moved,
            Err(err) => {
                warn!(
                    "❌ {name} in {} was not moved to refs: {err}",
                    source.display()
                );
                migrated.skipped.insert(name, err);
                continue;
            }
        };
        let held = PackageLock::take(storage, paths, &namespace).await?;
        if store.contains(storage, &namespace).await {
            continue;
        }
        store
            .create_moved(storage, &held, &new_ref, &lineage)
            .await?;
        crate::failpoint::hit("migrate.up.after-ref");
        migrated.written.insert(namespace);
    }
    Ok(migrated)
}

/// The ref of the entry `name`, carrying the keys [`PackageLineage`] does
/// not read, and the lineage it holds. Refused where that ref would not
/// read back, as when such a key is one the ref itself uses.
fn moved(
    paths: &DomainPaths,
    name: &str,
    entry: Value,
) -> Res<(Namespace, PackageRef, PackageLineage)> {
    let namespace = Namespace::try_from(name)?;
    let lineage = PackageLineage::deserialize(&entry)
        .map_err(|err| Error::Lineage(LineageError::Parse(err)))?;
    let mut new_ref = PackageRef::from_lineage(&lineage, None)?;
    if let Value::Object(entry) = entry {
        new_ref.extra = entry
            .into_iter()
            .filter(|(key, _)| !ENTRY_KEYS.contains(&key.as_str()))
            .collect();
    }
    PackageRef::from_slice(&paths.package_ref(&namespace), &new_ref.to_vec()?)?;
    Ok((namespace, new_ref, lineage))
}

async fn finish(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res {
    {
        let _domain = storage.lock_exclusive(paths.domain_file_lock()).await?;
        let mut config = read_config(storage, paths)
            .await?
            .ok_or(LineageError::Missing)?;
        config.migration = Some(MigrationState::Done);
        put(storage, &paths.domain_config(), config.to_vec()?, None).await?;
    }
    remove_working_copy(storage, paths).await
}

/// The first move's copy, `data.json.pre-refs`, stays for good.
async fn remove_working_copy(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res {
    if remove_if_present(storage, &paths.lineage_migrating()).await? {
        storage.sync_dir(paths.dot_quilt_dir()).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use test_log::test;

    use super::*;
    use crate::io::storage::LocalStorage;
    use crate::lineage::CommitState;
    use crate::lineage::DomainLineageIo;
    use crate::lineage::SyncScope;
    use crate::lineage::refs::RefEntry;
    use crate::lineage::refs::format::Extra;
    use crate::lineage::refs::format::TipCache;
    use quilt_uri::ManifestUri;

    const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const REMOTE: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const TIP: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const COMMIT: &str = "4444444444444444444444444444444444444444444444444444444444444444";
    const TIMESTAMP: &str = "2026-11-12T20:00:00Z";

    fn data() -> Namespace {
        ("acme", "data").into()
    }

    fn notes() -> Namespace {
        ("other", "notes").into()
    }

    /// A pushed package holding a key this version does not read, and a
    /// package never pushed, with a commit pending.
    fn legacy() -> Value {
        json!({
            "home": "/home",
            "packages": {
                "acme/data": {
                    "commit": null,
                    "remote": {"origin": null, "bucket": "b", "namespace": "acme/data", "hash": REMOTE},
                    "base_hash": BASE,
                    "latest_hash": TIP,
                    "paths": {
                        "a.csv": {"timestamp": TIMESTAMP},
                        "data/b.csv": {"timestamp": TIMESTAMP}
                    },
                    "sync_scope": "entire_package",
                    "future": {"kept": true}
                },
                "other/notes": {
                    "commit": {"timestamp": TIMESTAMP, "hash": COMMIT, "prev_hashes": [BASE]},
                    "base_hash": "",
                    "latest_hash": "",
                    "paths": {"notes.txt": {"timestamp": TIMESTAMP}}
                }
            }
        })
    }

    /// [`legacy`] after an older build uninstalled `other/notes`.
    fn legacy_without_notes() -> Value {
        let mut lineage = legacy();
        lineage["packages"]
            .as_object_mut()
            .expect("packages")
            .remove("other/notes");
        lineage
    }

    fn bytes(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).expect("JSON")
    }

    fn in_progress() -> Vec<u8> {
        bytes(&json!({"format": 1, "home": "/home", "migration": "in_progress"}))
    }

    fn write(path: &Path, bytes: &[u8]) -> Res {
        std::fs::create_dir_all(path.parent().expect("a folder"))?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn read(path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    struct Domain {
        _root: tempfile::TempDir,
        paths: DomainPaths,
        storage: LocalStorage,
    }

    impl Domain {
        fn new() -> Res<Self> {
            let root = tempfile::TempDir::new()?;
            let paths = DomainPaths::new(root.path().to_path_buf());
            std::fs::create_dir_all(paths.dot_quilt_dir())?;
            Ok(Self {
                _root: root,
                paths,
                storage: LocalStorage::new(),
            })
        }

        /// A domain whose `data.json` is `lineage`.
        fn on(lineage: &Value) -> Res<Self> {
            let domain = Self::new()?;
            write(&domain.paths.lineage(), &bytes(lineage))?;
            Ok(domain)
        }

        async fn up(&self) -> Res<Migrated> {
            up(&self.storage, &self.paths).await
        }

        fn ref_json(&self, namespace: &Namespace) -> Res<Value> {
            Ok(serde_json::from_slice(&std::fs::read(
                self.paths.package_ref(namespace),
            )?)?)
        }

        /// Each ref's bytes, by its path under `refs/`.
        fn refs(&self) -> Res<BTreeMap<PathBuf, Vec<u8>>> {
            let mut refs = BTreeMap::new();
            let root = self.paths.refs_dir();
            if !root.exists() {
                return Ok(refs);
            }
            for owner in std::fs::read_dir(&root)? {
                for file in std::fs::read_dir(owner?.path())? {
                    let path = file?.path();
                    if let RefEntry::Ref(_) = RefEntry::classify(path.file_name().expect("a name"))
                    {
                        let relative = path.strip_prefix(&root).expect("under refs/");
                        refs.insert(relative.to_path_buf(), std::fs::read(&path)?);
                    }
                }
            }
            Ok(refs)
        }

        fn put_refs(&self, refs: &BTreeMap<PathBuf, Vec<u8>>) -> Res {
            for (path, bytes) in refs {
                write(&self.paths.refs_dir().join(path), bytes)?;
            }
            Ok(())
        }

        fn config(&self) -> Res<DomainConfig> {
            let path = self.paths.domain_config();
            DomainConfig::from_slice(&path, &std::fs::read(&path)?)
        }

        /// Checks the move is done: the stub in place of `data.json`, the
        /// config at `home` saying so, and no working copy left.
        fn assert_done(&self, home: &str) -> Res {
            assert!(state::is_stub(&std::fs::read(self.paths.lineage())?));
            let config = self.config()?;
            assert_eq!(config.migration, Some(MigrationState::Done));
            assert_eq!(config.home, Home::from(home));
            assert!(!self.paths.lineage_migrating().exists());
            Ok(())
        }
    }

    /// The refs [`legacy`] moves to, from an uninterrupted move.
    async fn moved_legacy() -> Res<BTreeMap<PathBuf, Vec<u8>>> {
        let domain = Domain::on(&legacy())?;
        domain.up().await?;
        domain.refs()
    }

    #[test(tokio::test)]
    async fn each_entry_becomes_a_ref_carrying_the_keys_lineage_does_not_read() -> Res {
        let domain = Domain::on(&legacy())?;

        let migrated = domain.up().await?;

        assert_eq!(migrated.written, BTreeSet::from([data(), notes()]));
        assert!(migrated.skipped.is_empty());
        assert_eq!(
            domain.ref_json(&data())?,
            json!({
                "format": 1,
                "remote": {"origin": null, "bucket": "b", "namespace": "acme/data", "hash": REMOTE},
                "base_hash": BASE,
                "copies": [{"sync_scope": "entire_package", "tracked": ["a.csv", "data/b.csv"]}],
                "future": {"kept": true}
            })
        );
        assert_eq!(
            domain.ref_json(&notes())?,
            json!({
                "format": 1,
                "commit": {"timestamp": TIMESTAMP, "hash": COMMIT, "prev_hashes": [BASE]},
                "copies": [{"sync_scope": "individual_files", "tracked": ["notes.txt"]}]
            })
        );
        let tip = TipCache::from_slice(&std::fs::read(domain.paths.package_tip(&data()))?);
        assert_eq!(tip.map(|tip| tip.latest_hash), Some(TIP.to_string()));
        assert!(!domain.paths.package_tip(&notes()).exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_move_keeps_data_json_as_it_was_and_puts_the_stub_in_its_place() -> Res {
        let domain = Domain::on(&legacy())?;

        domain.up().await?;

        assert_eq!(
            read(&domain.paths.lineage_pre_refs()),
            Some(bytes(&legacy()))
        );
        domain.assert_done("/home")
    }

    /// A domain no command has used is moved too, so an older build finds
    /// the stub rather than creating an empty `data.json` beside the refs.
    #[test(tokio::test)]
    async fn a_fresh_domain_is_moved_with_no_home_and_no_package() -> Res {
        let domain = Domain::new()?;

        let migrated = domain.up().await?;

        assert!(migrated.written.is_empty());
        domain.assert_done("")?;
        let frozen = read(&domain.paths.lineage_pre_refs()).expect("a frozen copy");
        assert!(
            Frozen::from_slice(Path::new("pre-refs"), &frozen)?
                .packages
                .is_empty()
        );
        assert!(domain.refs()?.is_empty());
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_entry_that_does_not_parse_is_skipped_by_name_and_kept_in_the_frozen_copy() -> Res {
        let mut lineage = legacy();
        let mut clashing = lineage["packages"]["other/notes"].clone();
        clashing["format"] = json!(2);
        let valid = clashing.clone();
        let packages = lineage["packages"].as_object_mut().expect("packages");
        packages.insert("bad/entry".into(), json!({"paths": "damaged"}));
        packages.insert("clash/format".into(), clashing);
        packages.insert("no-namespace".into(), valid);
        let domain = Domain::on(&lineage)?;

        let migrated = domain.up().await?;

        assert_eq!(
            migrated.skipped.keys().collect::<Vec<_>>(),
            ["bad/entry", "clash/format", "no-namespace"]
        );
        assert_eq!(migrated.written, BTreeSet::from([data(), notes()]));
        assert_eq!(domain.refs()?, moved_legacy().await?);
        assert_eq!(
            read(&domain.paths.lineage_pre_refs()),
            Some(bytes(&lineage))
        );
        domain.assert_done("/home")
    }

    #[test(tokio::test)]
    async fn a_data_json_that_does_not_parse_is_refused_and_nothing_changes() -> Res {
        let domain = Domain::new()?;
        write(&domain.paths.lineage(), b"{\"home\": \"/home\", \"pack")?;

        let refused = domain.up().await;

        assert!(
            matches!(
                &refused,
                Err(Error::Lineage(LineageError::MigrationRefused { path, .. })) if *path == domain.paths.lineage()
            ),
            "{refused:?}"
        );
        assert_eq!(
            read(&domain.paths.lineage()),
            Some(b"{\"home\": \"/home\", \"pack".to_vec())
        );
        assert!(!domain.paths.lineage_pre_refs().exists());
        assert!(!domain.paths.domain_config().exists());
        Ok(())
    }

    /// Validation item 5's resume, from what a kill at each step leaves,
    /// built directly: each ends with the refs an uninterrupted move writes.
    #[test(tokio::test)]
    async fn a_move_stopped_at_each_step_resumes_to_the_same_refs() -> Res {
        let expected = moved_legacy().await?;
        let first_ref = BTreeMap::from_iter(
            expected
                .first_key_value()
                .map(|(path, bytes)| (path.clone(), bytes.clone())),
        );
        let steps = [
            "after-pre-refs",
            "after-stub",
            "after-config",
            "after-ref",
            "before-done",
        ];
        for (at, step) in steps.into_iter().enumerate() {
            let domain = Domain::on(&legacy())?;
            let paths = &domain.paths;
            write(&paths.lineage_pre_refs(), &bytes(&legacy()))?;
            if at >= 1 {
                write(&paths.lineage(), state::STUB)?;
            }
            if at >= 2 {
                write(&paths.domain_config(), &in_progress())?;
            }
            if at == 3 {
                domain.put_refs(&first_ref)?;
            }
            if at == 4 {
                domain.put_refs(&expected)?;
            }

            domain.up().await?;

            assert_eq!(domain.refs()?, expected, "{step}");
            assert_eq!(
                read(&paths.lineage_pre_refs()),
                Some(bytes(&legacy())),
                "{step}"
            );
            domain.assert_done("/home")?;
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_ref_an_interrupted_move_wrote_is_not_rewritten() -> Res {
        let domain = Domain::on(&legacy())?;
        write(&domain.paths.lineage_pre_refs(), &bytes(&legacy()))?;
        write(&domain.paths.lineage(), state::STUB)?;
        write(&domain.paths.domain_config(), &in_progress())?;
        let written = bytes(&json!({
            "format": 1,
            "copies": [{"sync_scope": "individual_files", "tracked": []}],
            "written": "before the kill"
        }));
        write(&domain.paths.package_ref(&data()), &written)?;

        let migrated = domain.up().await?;

        assert_eq!(migrated.written, BTreeSet::from([notes()]));
        assert_eq!(read(&domain.paths.package_ref(&data())), Some(written));
        Ok(())
    }

    /// No ref is written before the domain config exists, so refs found
    /// with none were not written from this `data.json`.
    #[test(tokio::test)]
    async fn refs_found_before_the_move_began_are_set_aside() -> Res {
        let domain = Domain::on(&legacy())?;
        let stale = bytes(&json!({
            "format": 1,
            "copies": [{"sync_scope": "individual_files", "tracked": []}]
        }));
        write(&domain.paths.package_ref(&data()), &stale)?;

        domain.up().await?;

        assert_eq!(domain.refs()?, moved_legacy().await?);
        let aside: Vec<PathBuf> = std::fs::read_dir(domain.paths.dot_quilt_dir())?
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.to_string_lossy().contains("refs.set-aside-"))
            .collect();
        let [aside] = aside.as_slice() else {
            panic!("{aside:?}");
        };
        assert_eq!(read(&aside.join("acme").join("data.json")), Some(stale));
        Ok(())
    }

    /// After a move and a rollback, the next move freezes its own copy and
    /// leaves the first move's as it was.
    #[test(tokio::test)]
    async fn a_second_move_keeps_the_first_frozen_copy() -> Res {
        let domain = Domain::on(&legacy_without_notes())?;
        write(&domain.paths.lineage_pre_refs(), &bytes(&legacy()))?;
        std::fs::create_dir(domain.paths.refs_archive("refs.rolled-back-x"))?;

        let migrated = domain.up().await?;

        assert_eq!(migrated.written, BTreeSet::from([data()]));
        assert_eq!(
            read(&domain.paths.lineage_pre_refs()),
            Some(bytes(&legacy()))
        );
        domain.assert_done("/home")
    }

    #[test(tokio::test)]
    async fn a_second_move_resumes_from_its_own_copy() -> Res {
        let domain = Domain::new()?;
        let paths = &domain.paths;
        write(&paths.lineage(), state::STUB)?;
        write(&paths.lineage_pre_refs(), &bytes(&legacy()))?;
        write(&paths.lineage_migrating(), &bytes(&legacy_without_notes()))?;
        write(&paths.domain_config(), &in_progress())?;

        let migrated = domain.up().await?;

        assert_eq!(migrated.written, BTreeSet::from([data()]));
        assert!(!paths.package_ref(&notes()).exists());
        assert_eq!(read(&paths.lineage_pre_refs()), Some(bytes(&legacy())));
        domain.assert_done("/home")
    }

    #[test(tokio::test)]
    async fn a_domain_on_refs_is_left_as_it_is_but_for_a_leftover_working_copy() -> Res {
        let domain = Domain::new()?;
        let paths = &domain.paths;
        let done = bytes(&json!({"format": 1, "home": "/home", "migration": "done"}));
        write(&paths.lineage(), state::STUB)?;
        write(&paths.domain_config(), &done)?;
        write(&paths.lineage_migrating(), &bytes(&legacy()))?;

        let migrated = domain.up().await?;

        assert!(migrated.written.is_empty());
        assert_eq!(read(&paths.domain_config()), Some(done));
        assert!(domain.refs()?.is_empty());
        domain.assert_done("/home")
    }

    #[test(tokio::test)]
    async fn a_domain_config_keeps_the_keys_this_version_does_not_read() -> Res {
        let domain = Domain::on(&legacy())?;
        write(
            &domain.paths.domain_config(),
            &bytes(&json!({"format": 1, "home": "/old", "future": true})),
        )?;

        domain.up().await?;

        assert_eq!(
            domain.config()?.extra,
            Extra::from_iter([("future".to_string(), json!(true))])
        );
        domain.assert_done("/home")
    }

    #[test(tokio::test)]
    async fn a_domain_being_rolled_back_is_refused() -> Res {
        let domain = Domain::new()?;
        let paths = &domain.paths;
        let rolling_back = bytes(&json!({
            "format": 1,
            "home": "/home",
            "migration": {"rolling_back": {"archive": state::archive_name(Uuid::new_v4())}}
        }));
        write(&paths.lineage(), state::STUB)?;
        write(&paths.domain_config(), &rolling_back)?;

        let refused = domain.up().await;

        assert!(
            matches!(
                &refused,
                Err(Error::Lineage(LineageError::RollingBack { path })) if *path == paths.domain_config()
            ),
            "{refused:?}"
        );
        assert_eq!(read(&paths.domain_config()), Some(rolling_back));
        assert_eq!(read(&paths.lineage()), Some(state::STUB.to_vec()));
        Ok(())
    }

    /// A build on `data.json` writes it under the short lock, so the move,
    /// which freezes it under that lock, never copies it mid-write.
    #[test(tokio::test)]
    async fn the_move_waits_for_a_build_on_data_json_writing_it() -> Res {
        let domain = Domain::on(&legacy())?;
        let short = DomainLineageIo::new(domain.paths.lineage()).short_lock_path();
        let held = domain.storage.lock_exclusive(&short).await?;
        let moving = domain.up();
        tokio::pin!(moving);

        let waited = tokio::time::timeout(Duration::from_millis(300), &mut moving).await;
        assert!(waited.is_err(), "the move did not wait");
        assert_eq!(read(&domain.paths.lineage()), Some(bytes(&legacy())));
        drop(held);
        moving.await?;

        domain.assert_done("/home")
    }

    /// The structure is the contract: a key `PackageLineage` writes is not
    /// carried into the ref as one it does not read.
    #[test]
    fn the_entry_keys_are_those_a_lineage_writes() -> Res {
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            remote_uri: Some(ManifestUri::default()),
            sync_scope: SyncScope::EntirePackage,
            ..PackageLineage::default()
        };
        let Value::Object(entry) = serde_json::to_value(&lineage)? else {
            panic!("an object");
        };
        assert_eq!(
            entry.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            BTreeSet::from(ENTRY_KEYS)
        );
        Ok(())
    }
}
