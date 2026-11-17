//!
//! The down-migration: hands a domain's lineage back from the refs to
//! `data.json`, in steps a killed run resumes from what it finds.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "`quilt lineage rollback` calls it")
)]

use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use serde_json::Map;
use serde_json::Value;
use uuid::Uuid;

use super::RefStore;
use super::format::MigrationState;
use super::format::PackageRef;
use super::format::TipCache;
use super::state;
use super::state::DomainState;
use super::store::put;
use super::store::read_config;
use super::store::remove_if_present;
use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::Storage;
use crate::lineage::Home;
use crate::package_lock::PackageLock;
use crate::paths;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// A finished rollback.
#[derive(Debug)]
pub(crate) struct RolledBack {
    /// Where the refs are kept.
    pub archive: PathBuf,
}

/// Hands the domain's lineage back to `data.json`, or finishes a rollback a
/// kill interrupted. While any ref holds what `data.json` cannot, it refuses
/// with [`LineageError::RollbackRefused`] and leaves the refs writable.
pub(crate) async fn down(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res<RolledBack> {
    let _migrating = storage.lock_exclusive(paths.migrate_lock()).await?;
    let store = RefStore::new(paths.clone());
    let (archive, archived, published) = match state::detect(storage, paths).await? {
        DomainState::Refs => {
            for namespace in store.listed(storage).await? {
                read(storage, &store, &namespace).await?;
            }
            let archive = begin(storage, paths).await?;
            crate::failpoint::hit("migrate.down.after-state");
            (archive, false, false)
        }
        DomainState::RollingBack {
            archive,
            archived,
            published,
        } => (archive, archived, published),
        DomainState::Legacy => return Err(not_on_refs(paths, "has not started")),
        DomainState::StubOnly | DomainState::Migrating => {
            return Err(not_on_refs(paths, "is in progress"));
        }
    };
    if !published {
        if !archived {
            stage(storage, paths, &store).await?;
            crate::failpoint::hit("migrate.down.after-staged");
            archive_refs(storage, paths, &archive).await?;
            crate::failpoint::hit("migrate.down.after-archive");
        }
        publish(storage, paths).await?;
        crate::failpoint::hit("migrate.down.after-publish");
    }
    remove_if_present(storage, &paths.domain_config()).await?;
    storage.sync_dir(paths.dot_quilt_dir()).await;
    Ok(RolledBack { archive })
}

fn not_on_refs(paths: &DomainPaths, state: &'static str) -> Error {
    LineageError::RollbackRefused(Box::new(LineageError::RefsNotWritable {
        path: paths.domain_config(),
        state,
    }))
    .into()
}

/// Step 1: stops every ref writer and names the archive.
async fn begin(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res<PathBuf> {
    let name = state::archive_name(Uuid::new_v4());
    set_migration(
        storage,
        paths,
        MigrationState::RollingBack {
            archive: name.clone(),
        },
    )
    .await?;
    Ok(paths.refs_archive(&name))
}

async fn set_migration(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    migration: MigrationState,
) -> Res {
    let _domain = storage.lock_exclusive(paths.domain_file_lock()).await?;
    let mut config = read_config(storage, paths)
        .await?
        .ok_or(LineageError::Missing)?;
    config.migration = Some(migration);
    put(storage, &paths.domain_config(), config.to_vec()?, None).await
}

/// Step 2: the legacy file from the refs, staged. A ref that came to hold
/// what `data.json` cannot since the check before step 1 sends the domain
/// back to `done`.
async fn stage(storage: &(impl Storage + Sync), paths: &DomainPaths, store: &RefStore) -> Res {
    let staged = paths.lineage_rollback();
    match legacy_file(storage, paths, store).await {
        Ok(bytes) => storage.write_byte_stream(&staged, bytes.into()).await,
        Err(err @ Error::Lineage(LineageError::RollbackRefused(_))) => {
            remove_if_present(storage, &staged).await?;
            storage.sync_dir(paths.dot_quilt_dir()).await;
            set_migration(storage, paths, MigrationState::Done).await?;
            Err(err)
        }
        Err(err) => Err(err),
    }
}

/// `data.json` as the refs hold it, each ref read under its package's lock so
/// a write in flight lands first.
async fn legacy_file(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    store: &RefStore,
) -> Res<Vec<u8>> {
    let home = store.home(storage).await?;
    let now = Utc::now();
    let mut packages = Map::new();
    for namespace in store.listed(storage).await? {
        let read = {
            let _held = PackageLock::take(storage, paths, &namespace).await?;
            read(storage, store, &namespace).await?
        };
        if let Some((stored, tip)) = read {
            let entry = entry(storage, &home, &namespace, stored, tip, now).await?;
            packages.insert(namespace.to_string(), entry);
        }
    }
    let record = serde_json::json!({"home": home, "packages": packages});
    Ok(serde_json::to_vec_pretty(&record)?)
}

/// The package's ref and tip; `None` once it is gone. A ref that does not
/// parse, or that `data.json` cannot hold, refuses the rollback.
async fn read(
    storage: &(impl Storage + Sync),
    store: &RefStore,
    namespace: &Namespace,
) -> Res<Option<(PackageRef, Option<TipCache>)>> {
    match store.read_ref(storage, namespace).await {
        Ok(read) => Ok(Some(read)),
        Err(Error::InstallPackage(InstallPackageError::NotInstalled(_))) => Ok(None),
        Err(Error::Lineage(err)) => Err(LineageError::RollbackRefused(Box::new(err)).into()),
        Err(err) => Err(err),
    }
}

/// The package's `data.json` entry. Each path is timed by its working file,
/// else by `now`, and has no hash, which the ref does not keep; the ref's
/// unknown keys become the entry's.
async fn entry(
    storage: &(impl Storage + Sync),
    home: &Home,
    namespace: &Namespace,
    mut stored: PackageRef,
    tip: Option<TipCache>,
    now: DateTime<Utc>,
) -> Res<Value> {
    let extra = std::mem::take(&mut stored.extra);
    let mut lineage = stored.into_lineage(tip.as_ref());
    let folder = paths::package_home(home, namespace);
    for (key, path) in &mut lineage.paths {
        path.timestamp = storage
            .modified_timestamp(folder.join(key))
            .await
            .unwrap_or(now);
    }
    let mut entry = serde_json::to_value(lineage)?;
    if let Some(entry) = entry.as_object_mut() {
        for (key, value) in extra {
            // The entry leaves out a default scope, which the copy's scope,
            // not a stray top-level key, stands for.
            if key != "sync_scope" {
                entry.entry(key).or_insert(value);
            }
        }
    }
    Ok(entry)
}

/// Step 3: an empty archive where there are no refs, so the step always
/// leaves one.
async fn archive_refs(storage: &(impl Storage + Sync), paths: &DomainPaths, archive: &Path) -> Res {
    let refs = paths.refs_dir();
    if storage.exists(&refs).await {
        moved(storage, &refs, archive).await?;
    } else {
        storage.create_dir_all(archive).await?;
    }
    storage.sync_dir(paths.dot_quilt_dir()).await;
    Ok(())
}

/// Step 4: the staged file over the stub, under the short lock older builds
/// write `data.json` under.
async fn publish(storage: &(impl Storage + Sync), paths: &DomainPaths) -> Res {
    let _short = storage.lock_exclusive(paths.lineage_lock()).await?;
    moved(storage, &paths.lineage_rollback(), &paths.lineage()).await?;
    storage.sync_dir(paths.dot_quilt_dir()).await;
    Ok(())
}

/// On Windows a rename fails while another process has a file open, so the
/// error names both files; the rollback resumes once run again.
async fn moved(storage: &(impl Storage + Sync), from: &Path, to: &Path) -> Res {
    storage.rename(from, to).await.map_err(|err| {
        let kind = match &err {
            Error::Io(err) => err.kind(),
            _ => std::io::ErrorKind::Other,
        };
        std::io::Error::new(
            kind,
            format!(
            "Could not move {} to {}: {err}. If another program has either open, close it; rolling back again resumes here.",
            from.display(),
            to.display(),
            ),
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use test_log::test;

    use super::*;
    use crate::io::storage::LocalStorage;
    use crate::lineage::DomainLineageIo;
    use crate::lineage::PackageLineage;
    use crate::lineage::refs::state::STUB;

    const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const TIP: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const COMMIT: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const PRE_REFS: &[u8] = br#"{"home": "/before", "packages": {}}"#;
    const STALE: &[u8] = br#"{"home": "/stale", "packages": {}}"#;

    /// A domain as the cut-over leaves it: the stub in place of `data.json`,
    /// the first `data.json` kept, the config `done`, and no ref yet.
    struct Domain {
        _root: tempfile::TempDir,
        paths: DomainPaths,
        home: PathBuf,
        storage: LocalStorage,
    }

    impl Domain {
        fn new() -> Res<Self> {
            let root = tempfile::TempDir::new()?;
            let paths = DomainPaths::new(root.path().join("domain"));
            let home = root.path().join("home");
            std::fs::create_dir_all(paths.dot_quilt_dir())?;
            std::fs::create_dir_all(&home)?;
            std::fs::write(paths.lineage(), STUB)?;
            std::fs::write(paths.lineage_pre_refs(), PRE_REFS)?;
            let domain = Self {
                _root: root,
                paths,
                home,
                storage: LocalStorage::new(),
            };
            domain.set_config(&json!("done"))?;
            Ok(domain)
        }

        fn set_config(&self, migration: &Value) -> Res {
            let config = json!({"format": 1, "home": self.home, "migration": migration});
            Ok(std::fs::write(
                self.paths.domain_config(),
                serde_json::to_vec(&config)?,
            )?)
        }

        /// The config as a killed step 1 leaves it.
        fn set_rolling_back(&self) -> Res<PathBuf> {
            let name = state::archive_name(Uuid::new_v4());
            self.set_config(&json!({"rolling_back": {"archive": name}}))?;
            Ok(self.paths.refs_archive(&name))
        }

        fn put_ref(&self, name: &str, value: &Value) -> Res {
            write(&self.paths.package_ref(&package(name)), value)
        }

        fn put_tip(&self, name: &str, of: &str) -> Res {
            let tip = json!({
                "format": 1,
                "latest_hash": TIP,
                "bucket": "b",
                "namespace": format!("acme/{of}"),
                "checked_at": "2026-11-01T00:00:00Z",
            });
            write(&self.paths.package_tip(&package(name)), &tip)
        }

        async fn down(&self) -> Res<RolledBack> {
            down(&self.storage, &self.paths).await
        }

        fn legacy(&self) -> Res<Value> {
            Ok(serde_json::from_slice(&std::fs::read(
                self.paths.lineage(),
            )?)?)
        }

        /// Authority is back with `data.json`, the refs in `archive`, and
        /// the first `data.json` as the cut-over kept it.
        fn assert_rolled_back(&self, archive: &Path) -> Res {
            assert!(!self.paths.domain_config().exists());
            assert!(!self.paths.lineage_rollback().exists());
            assert!(!self.paths.refs_dir().exists());
            assert!(archive.is_dir());
            assert_eq!(std::fs::read(self.paths.lineage_pre_refs())?, PRE_REFS);
            Ok(())
        }

        /// Nothing a rollback writes is there, and the refs are writable.
        async fn assert_on_refs(&self) -> Res {
            assert_eq!(std::fs::read(self.paths.lineage())?, STUB);
            assert!(!self.paths.lineage_rollback().exists());
            assert_eq!(archives(&self.paths)?, Vec::<PathBuf>::new());
            assert_eq!(std::fs::read(self.paths.lineage_pre_refs())?, PRE_REFS);
            let namespace = package("new");
            let held = PackageLock::take(&self.storage, &self.paths, &namespace).await?;
            RefStore::new(self.paths.clone())
                .create(
                    &self.storage,
                    &held,
                    &PackageLineage::default(),
                    &Home::from(&self.home),
                )
                .await
        }
    }

    fn write(path: &Path, value: &Value) -> Res {
        std::fs::create_dir_all(path.parent().expect("a folder"))?;
        Ok(std::fs::write(path, serde_json::to_vec(value)?)?)
    }

    fn package(name: &str) -> Namespace {
        ("acme", name).into()
    }

    fn remote(name: &str, hash: &str) -> Value {
        json!({"origin": null, "bucket": "b", "namespace": format!("acme/{name}"), "hash": hash})
    }

    /// A ref pushed at [`BASE`], tracking `a.csv`.
    fn pushed(name: &str) -> Value {
        json!({
            "format": 1,
            "remote": remote(name, BASE),
            "base_hash": BASE,
            "copies": [{"sync_scope": "individual_files", "tracked": ["a.csv"]}],
        })
    }

    fn archives(paths: &DomainPaths) -> Res<Vec<PathBuf>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(paths.dot_quilt_dir())? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("refs.rolled-back-"))
            {
                found.push(path);
            }
        }
        Ok(found)
    }

    #[test(tokio::test)]
    async fn the_legacy_file_rebuilds_what_the_refs_leave_out_by_rule() -> Res {
        let domain = Domain::new()?;
        let mut tipped = pushed("tipped");
        tipped["commit"] =
            json!({"hash": COMMIT, "prev_hashes": [BASE], "timestamp": "2026-11-12T20:00:00Z"});
        tipped["copies"] =
            json!([{"sync_scope": "entire_package", "tracked": ["a.csv", "data/gone.csv"]}]);
        tipped["future"] = json!({"nested": [1, 2]});
        domain.put_ref("tipped", &tipped)?;
        domain.put_tip("tipped", "tipped")?;
        domain.put_ref("other-tip", &pushed("other-tip"))?;
        domain.put_tip("other-tip", "elsewhere")?;
        domain.put_ref("untipped", &pushed("untipped"))?;
        domain.put_ref(
            "local",
            &json!({
                "format": 1,
                "copies": [{"tracked": ["a.csv"]}],
                "sync_scope": "entire_package",
            }),
        )?;
        domain.put_ref(
            "unpushed",
            &json!({"format": 1, "remote": remote("unpushed", ""), "copies": [{"tracked": []}]}),
        )?;
        let file = domain.home.join("acme/tipped/a.csv");
        std::fs::create_dir_all(file.parent().expect("a folder"))?;
        std::fs::write(&file, b"a")?;
        let mtime: DateTime<Utc> = "2026-01-02T03:04:05Z".parse().expect("a time");
        std::fs::File::options()
            .write(true)
            .open(&file)?
            .set_modified(mtime.into())?;

        let before = Utc::now();
        let rolled = domain.down().await?;
        let after = Utc::now();

        domain.assert_rolled_back(&rolled.archive)?;
        let legacy = domain.legacy()?;
        assert_eq!(legacy["home"], json!(domain.home));
        let entry = |name: &str| legacy["packages"][format!("acme/{name}")].clone();
        let latest: Vec<(&str, Value, Value)> =
            ["tipped", "other-tip", "untipped", "local", "unpushed"]
                .into_iter()
                .map(|name| {
                    (
                        name,
                        entry(name)["base_hash"].clone(),
                        entry(name)["latest_hash"].clone(),
                    )
                })
                .collect();
        assert_eq!(
            latest,
            [
                ("tipped", json!(BASE), json!(TIP)),
                ("other-tip", json!(BASE), json!(BASE)),
                ("untipped", json!(BASE), json!(BASE)),
                ("local", json!(""), json!("")),
                ("unpushed", json!(""), json!("")),
            ]
        );
        let tipped = entry("tipped");
        assert_eq!(tipped["commit"]["hash"], json!(COMMIT));
        assert_eq!(tipped["remote"], remote("tipped", BASE));
        assert_eq!(tipped["sync_scope"], json!("entire_package"));
        assert_eq!(tipped["future"], json!({"nested": [1, 2]}));
        // The copy's default scope, not a stray top-level one.
        assert_eq!(entry("local").get("sync_scope"), None);
        let paths = tipped["paths"].as_object().expect("paths");
        assert_eq!(paths.keys().collect::<Vec<_>>(), ["a.csv", "data/gone.csv"]);
        assert!(
            paths.values().all(|path| path.get("hash").is_none()),
            "{paths:?}"
        );
        let timestamp = |key: &str| -> Res<DateTime<Utc>> {
            Ok(serde_json::from_value(paths[key]["timestamp"].clone())?)
        };
        assert_eq!(timestamp("a.csv")?, mtime);
        let gone = timestamp("data/gone.csv")?;
        assert!(before <= gone && gone <= after, "{gone}");
        // The older client reads every entry.
        let read = DomainLineageIo::new(domain.paths.lineage())
            .read_packages(&domain.storage)
            .await?;
        assert_eq!(read.len(), 5);
        assert!(read.values().all(Result::is_ok), "{read:?}");
        Ok(())
    }

    /// The spec's tip-unknown rule is `base_hash`; the store's is
    /// `untipped_latest`, which the rollback uses.
    #[test]
    fn the_latest_hash_with_no_tip_is_the_base_hash_for_every_kind_of_ref() -> Res {
        for value in [
            pushed("untipped"),
            json!({"format": 1, "copies": [{"tracked": []}]}),
            json!({"format": 1, "remote": remote("unpushed", ""), "copies": [{"tracked": []}]}),
        ] {
            let stored = PackageRef::from_slice(Path::new("ref"), &serde_json::to_vec(&value)?)?;
            assert_eq!(stored.untipped_latest(), stored.base_hash, "{value}");
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_domain_with_no_refs_rolls_back_to_no_packages_and_an_empty_archive() -> Res {
        let domain = Domain::new()?;

        let rolled = domain.down().await?;

        domain.assert_rolled_back(&rolled.archive)?;
        assert_eq!(std::fs::read_dir(&rolled.archive)?.count(), 0);
        assert_eq!(domain.legacy()?["packages"], json!({}));
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_rollback_killed_after_step_one_resumes_from_the_refs() -> Res {
        let domain = Domain::new()?;
        domain.put_ref("x", &pushed("x"))?;
        let archive = domain.set_rolling_back()?;

        domain.down().await?;

        domain.assert_rolled_back(&archive)?;
        assert!(archive.join("acme/x.json").is_file());
        assert_eq!(
            domain.legacy()?["packages"]["acme/x"]["base_hash"],
            json!(BASE)
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_rollback_killed_after_staging_stages_again_from_the_refs() -> Res {
        let domain = Domain::new()?;
        domain.put_ref("x", &pushed("x"))?;
        let archive = domain.set_rolling_back()?;
        std::fs::write(domain.paths.lineage_rollback(), STALE)?;

        domain.down().await?;

        domain.assert_rolled_back(&archive)?;
        assert_eq!(
            domain.legacy()?["packages"]["acme/x"]["base_hash"],
            json!(BASE)
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_rollback_killed_after_archiving_publishes_what_it_staged() -> Res {
        let domain = Domain::new()?;
        domain.put_ref("x", &pushed("x"))?;
        let archive = domain.set_rolling_back()?;
        std::fs::write(domain.paths.lineage_rollback(), STALE)?;
        std::fs::rename(domain.paths.refs_dir(), &archive)?;

        domain.down().await?;

        domain.assert_rolled_back(&archive)?;
        assert_eq!(std::fs::read(domain.paths.lineage())?, STALE);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_rollback_killed_after_publishing_keeps_data_json_as_it_is() -> Res {
        let domain = Domain::new()?;
        let archive = domain.set_rolling_back()?;
        std::fs::create_dir(&archive)?;
        std::fs::write(domain.paths.lineage(), STALE)?;

        domain.down().await?;

        domain.assert_rolled_back(&archive)?;
        assert_eq!(std::fs::read(domain.paths.lineage())?, STALE);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_ref_data_json_cannot_hold_refuses_the_rollback_and_changes_nothing() -> Res {
        let mut folder = pushed("x");
        folder["copies"][0]["folder"] = json!("/elsewhere");
        let mut two = pushed("x");
        two["copies"] = json!([{"tracked": []}, {"tracked": []}]);
        let mut newer = pushed("x");
        newer["format"] = json!(2);
        for (damage, refusal) in [
            (Some(folder), "UnsupportedCopies"),
            (Some(two), "UnsupportedCopies"),
            (Some(newer), "UnsupportedFormat"),
            (None, "FileParse"),
        ] {
            let domain = Domain::new()?;
            domain.put_ref("good", &pushed("good"))?;
            match &damage {
                Some(value) => domain.put_ref("x", value)?,
                None => std::fs::write(domain.paths.package_ref(&package("x")), b"{")?,
            }
            let config = std::fs::read(domain.paths.domain_config())?;

            let refused = domain.down().await;

            let reason = match &refused {
                Err(Error::Lineage(LineageError::RollbackRefused(reason))) => format!("{reason:?}"),
                other => panic!("{refusal}: {other:?}"),
            };
            assert!(reason.starts_with(refusal), "{reason}");
            assert_eq!(std::fs::read(domain.paths.domain_config())?, config);
            domain.assert_on_refs().await?;
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_ref_found_unholdable_after_step_one_sends_the_domain_back_to_done() -> Res {
        let domain = Domain::new()?;
        domain.put_ref("good", &pushed("good"))?;
        std::fs::write(domain.paths.package_ref(&package("x")), b"{")?;
        domain.set_rolling_back()?;
        std::fs::write(domain.paths.lineage_rollback(), STALE)?;

        let refused = domain.down().await;

        assert!(
            matches!(
                refused,
                Err(Error::Lineage(LineageError::RollbackRefused(_)))
            ),
            "{refused:?}"
        );
        let config: Value = serde_json::from_slice(&std::fs::read(domain.paths.domain_config())?)?;
        assert_eq!(config["migration"], json!("done"));
        domain.assert_on_refs().await
    }

    #[test(tokio::test)]
    async fn an_archive_named_outside_a_rollbacks_names_is_refused_and_nothing_moves() -> Res {
        for name in ["", "refs", "..", "/tmp/refs", "refs.rolled-back-../../x"] {
            let domain = Domain::new()?;
            domain.put_ref("x", &pushed("x"))?;
            domain.set_config(&json!({"rolling_back": {"archive": name}}))?;

            let refused = domain.down().await;

            assert!(
                matches!(
                    &refused,
                    Err(Error::Lineage(LineageError::FileParse { path, .. })) if *path == domain.paths.domain_config()
                ),
                "{name:?}: {refused:?}"
            );
            assert!(domain.paths.package_ref(&package("x")).is_file());
            assert_eq!(std::fs::read(domain.paths.lineage())?, STUB);
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_domain_not_on_refs_has_nothing_to_roll_back() -> Res {
        for (migration, state) in [
            (None, "has not started"),
            (Some(json!("in_progress")), "is in progress"),
        ] {
            let domain = Domain::new()?;
            if let Some(migration) = migration {
                domain.set_config(&migration)?;
            } else {
                std::fs::remove_file(domain.paths.domain_config())?;
                std::fs::remove_file(domain.paths.lineage())?;
            }

            let refused = domain.down().await;

            assert!(
                matches!(
                    &refused,
                    Err(Error::Lineage(LineageError::RollbackRefused(reason)))
                        if matches!(**reason, LineageError::RefsNotWritable { state: found, .. } if found == state)
                ),
                "{state}: {refused:?}"
            );
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_move_that_fails_names_both_files_and_resumes_once_it_can() -> Res {
        let domain = Domain::new()?;
        let archive = domain.set_rolling_back()?;
        std::fs::create_dir(&archive)?;

        let failed = domain.down().await;

        let message = failed.expect_err("nothing is staged").to_string();
        assert!(message.contains("data.json.rollback"), "{message}");
        assert!(
            message.contains(&domain.paths.lineage().display().to_string()),
            "{message}"
        );
        std::fs::write(domain.paths.lineage_rollback(), STALE)?;
        domain.down().await?;
        domain.assert_rolled_back(&archive)?;
        assert_eq!(std::fs::read(domain.paths.lineage())?, STALE);
        Ok(())
    }
}
