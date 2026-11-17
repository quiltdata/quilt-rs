//! Validation item 5's round trips: the `data.json` files in
//! `fixtures/lineage/`, moved to refs by a domain opened on them and back by
//! its rollback.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
use test_log::test;

use crate::Res;
use crate::flow::UserMeta;
use crate::lineage::Home;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::local_domain::LocalDomain;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// The keys of an entry that `PackageLineage` reads.
const KNOWN: [&str; 6] = [
    "commit",
    "remote",
    "base_hash",
    "latest_hash",
    "paths",
    "sync_scope",
];

/// Each fixture's name and contents.
fn fixtures() -> Res<Vec<(String, Value)>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/lineage");
    let mut fixtures = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .to_string();
        fixtures.push((name, serde_json::from_slice(&std::fs::read(&path)?)?));
    }
    fixtures.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(fixtures)
}

/// The fixture `quilt-cli-0.40.2.json`, written by that release.
fn released() -> Res<Value> {
    Ok(fixtures()?
        .into_iter()
        .find(|(name, _)| name == "quilt-cli-0.40.2.json")
        .expect("the fixture")
        .1)
}

struct Domain {
    root: TempDir,
    paths: DomainPaths,
}

impl Domain {
    /// A domain whose `data.json` is `lineage`.
    fn on(lineage: &Value) -> Res<Self> {
        let root = TempDir::new()?;
        let paths = DomainPaths::new(root.path().join("domain"));
        std::fs::create_dir_all(paths.dot_quilt_dir())?;
        std::fs::write(paths.lineage(), serde_json::to_vec_pretty(lineage)?)?;
        Ok(Self { root, paths })
    }

    /// [`Self::on`] with the home moved into the test's folder, for a test
    /// that places files.
    fn housing(lineage: &Value) -> Res<Self> {
        let domain = Self::on(lineage)?;
        let mut lineage = lineage.clone();
        lineage["home"] = json!(domain.root.path().join("home"));
        std::fs::write(domain.paths.lineage(), serde_json::to_vec_pretty(&lineage)?)?;
        Ok(domain)
    }

    /// The domain as a build with refs opens it.
    fn on_refs(&self) -> LocalDomain {
        LocalDomain::new(self.root.path().join("domain")).with_ref_store()
    }

    /// The domain as the older client, a build on `data.json`, opens it.
    fn on_data_json(&self) -> LocalDomain {
        LocalDomain::new(self.root.path().join("domain"))
    }

    fn data_json(&self) -> Res<Value> {
        Ok(serde_json::from_slice(&std::fs::read(
            self.paths.lineage(),
        )?)?)
    }

    fn ref_json(&self, namespace: &str) -> Res<Value> {
        let namespace = Namespace::try_from(namespace)?;
        Ok(serde_json::from_slice(&std::fs::read(
            self.paths.package_ref(&namespace),
        )?)?)
    }

    /// The archives the domain's rollbacks left, by name.
    fn archives(&self) -> Res<Vec<PathBuf>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(self.paths.dot_quilt_dir())? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("refs.rolled-back-"))
            {
                found.push(path);
            }
        }
        found.sort();
        Ok(found)
    }
}

fn entries(lineage: &Value) -> Res<BTreeMap<String, Value>> {
    Ok(serde_json::from_value(lineage["packages"].clone())?)
}

/// `entry` as `PackageLineage` reads it, without the paths' hashes and
/// times, which the refs do not keep.
fn authoritative(entry: &Value) -> Res<PackageLineage> {
    let mut lineage: PackageLineage = serde_json::from_value(entry.clone())?;
    lineage
        .paths
        .values_mut()
        .for_each(|path| *path = PathState::default());
    Ok(lineage)
}

/// The keys of `entry` that `PackageLineage` does not read.
fn unknown(entry: &Value) -> BTreeMap<String, Value> {
    entry
        .as_object()
        .expect("an entry")
        .iter()
        .filter(|(key, _)| !KNOWN.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Checks that `back`, the `data.json` a rollback wrote between `when.0`
/// and `when.1`, holds what `original` held, but for the paths' hashes,
/// which it leaves out, and their times, which it takes from the working
/// files, and with none there from when it ran.
fn assert_round_trip(
    name: &str,
    original: &Value,
    back: &Value,
    when: (DateTime<Utc>, DateTime<Utc>),
) -> Res {
    // A missing home reads as an empty one, as `data.json`'s parser has it.
    let home = |lineage: &Value| {
        serde_json::from_value::<Home>(lineage.get("home").cloned().unwrap_or(json!("")))
    };
    assert_eq!(home(back)?, home(original)?, "{name}");
    let (original, back) = (entries(original)?, entries(back)?);
    assert_eq!(
        back.keys().collect::<Vec<_>>(),
        original.keys().collect::<Vec<_>>(),
        "{name}"
    );
    for (namespace, entry) in &original {
        let returned = &back[namespace];
        let at = format!("{name}: {namespace}");
        assert_eq!(authoritative(returned)?, authoritative(entry)?, "{at}");
        assert_eq!(unknown(returned), unknown(entry), "{at}");
        let paths: BTreeMap<String, Value> = serde_json::from_value(returned["paths"].clone())?;
        for (key, path) in paths {
            assert_eq!(path.get("hash"), None, "{at}: {key}");
            let timestamp: DateTime<Utc> = serde_json::from_value(path["timestamp"].clone())?;
            assert!(when.0 <= timestamp && timestamp <= when.1, "{at}: {key}");
        }
    }
    Ok(())
}

/// Each fixture's packages read from the refs as `data.json` held them, and
/// after the rollback `data.json` holds them again.
#[test(tokio::test)]
async fn each_fixture_moves_to_refs_and_back_keeping_what_it_held() -> Res {
    let fixtures = fixtures()?;
    let mut folder_lists = 0;
    for (_, lineage) in &fixtures {
        folder_lists += entries(lineage)?
            .values()
            .filter(|entry| entry["sync_scope"]["folders"].is_array())
            .count();
    }
    assert!(folder_lists > 0, "no fixture keeps a folder list");
    for (name, original) in &fixtures {
        let domain = Domain::on(original)?;
        let on_refs = domain.on_refs();

        let moved = on_refs.read_packages().await?;

        let mut expected = BTreeMap::new();
        for (namespace, entry) in entries(original)? {
            expected.insert(Namespace::try_from(namespace)?, authoritative(&entry)?);
        }
        let mut read = BTreeMap::new();
        for (namespace, lineage) in moved {
            read.insert(namespace, lineage?);
        }
        assert_eq!(read, expected, "{name}");

        let before = Utc::now();
        on_refs.rollback_lineage().await?;
        let after = Utc::now();

        assert_round_trip(name, original, &domain.data_json()?, (before, after))?;
    }
    Ok(())
}

/// A key of an entry that this build does not read is in the package's ref
/// after the move, and back in the entry after the rollback.
#[test(tokio::test)]
async fn an_entry_s_unknown_keys_survive_the_move_and_the_rollback() -> Res {
    let mut original = released()?;
    let kept = json!({"pinned": {"by": "someone", "at": [1, 2.5]}, "label": null});
    for (key, value) in kept.as_object().expect("keys") {
        original["packages"]["acme/local"][key] = value.clone();
    }
    let domain = Domain::on(&original)?;
    let on_refs = domain.on_refs();

    on_refs.list_installed_packages().await?;
    let stored = domain.ref_json("acme/local")?;
    on_refs.rollback_lineage().await?;

    for (key, value) in kept.as_object().expect("keys") {
        assert_eq!(stored.get(key), Some(value), "{key} in the ref");
    }
    let back = domain.data_json()?;
    assert_eq!(
        unknown(&back["packages"]["acme/local"]),
        unknown(&original["packages"]["acme/local"])
    );
    Ok(())
}

#[test(tokio::test)]
async fn two_rollbacks_with_an_upgrade_between_both_complete() -> Res {
    let original = released()?;
    let domain = Domain::on(&original)?;
    let first_data_json = std::fs::read(domain.paths.lineage())?;
    let mut rolled = Vec::new();

    for _ in 0..2 {
        let on_refs = domain.on_refs();
        on_refs.list_installed_packages().await?;
        rolled.push(on_refs.rollback_lineage().await?);
    }

    rolled.sort();
    assert_eq!(domain.archives()?, rolled);
    for archive in &rolled {
        assert!(archive.join("acme/local.json").is_file(), "{archive:?}");
    }
    assert!(!domain.paths.domain_config().exists());
    assert_eq!(
        std::fs::read(domain.paths.lineage_pre_refs())?,
        first_data_json
    );
    let back = domain.data_json()?;
    for (namespace, entry) in entries(&original)? {
        assert_eq!(
            authoritative(&back["packages"][&namespace])?,
            authoritative(&entry)?,
            "{namespace}"
        );
    }
    Ok(())
}

/// What the older client writes between a rollback and the next upgrade is
/// what the refs hold after it: no ref from before the rollback comes back.
#[test(tokio::test)]
async fn an_older_client_s_writes_after_a_rollback_are_in_the_refs_after_the_next_upgrade() -> Res {
    let domain = Domain::housing(&released()?)?;
    let first = domain.on_refs();
    first.list_installed_packages().await?;
    first.rollback_lineage().await?;
    let older = domain.on_data_json();
    let created: Namespace = ("acme", "created").into();
    older
        .create_package(created.clone(), None, Some("after".into()), UserMeta::Clear)
        .await?;
    older.uninstall_package(("acme", "scoped").into()).await?;
    let mut expected = BTreeMap::new();
    for (namespace, lineage) in older.read_packages().await? {
        let mut lineage = lineage?;
        lineage
            .paths
            .values_mut()
            .for_each(|path| *path = PathState::default());
        expected.insert(namespace, lineage);
    }

    let moved = domain.on_refs().read_packages().await?;

    let mut read = BTreeMap::new();
    for (namespace, lineage) in moved {
        read.insert(namespace, lineage?);
    }
    assert_eq!(
        read.keys().collect::<Vec<_>>(),
        [&("acme", "created").into(), &("acme", "local").into()]
    );
    assert_eq!(read, expected);
    Ok(())
}
