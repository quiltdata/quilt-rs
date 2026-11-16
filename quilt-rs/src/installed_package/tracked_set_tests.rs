//! The files a copy tracks, over random sequences under each sync scope,
//! against a model of each operation's rule as the evidence of
//! <https://github.com/quiltdata/quilt-specs/pull/292> states it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use test_log::test;

use super::older_revision_tests::BUCKET;
use super::older_revision_tests::publish_manifest;
use super::older_revision_tests::publish_row;
use super::*;
use crate::io::manifest::tag_latest;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineageIo;
use crate::local_domain::LocalDomain;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;

/// The logical keys a sequence draws from; `d/` is the folder the folders
/// scope keeps.
const KEYS: [&str; 6] = ["a.txt", "b.txt", "c.txt", "d/e.txt", "d/f.txt", "g.txt"];

type Rows = BTreeMap<String, Vec<u8>>;

/// Deterministic, so a failing sequence runs again the same.
struct Draw(u64);

impl Draw {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % n as u64).expect("below n")
    }

    fn pick<'a>(&mut self, from: &'a [String]) -> Option<&'a String> {
        (!from.is_empty()).then(|| &from[self.below(from.len())])
    }
}

/// What the spec says a copy holds.
#[derive(Debug, Default)]
struct Model {
    scope: Scope,
    /// The rows of the revision the copy is at.
    rows: Rows,
    /// The rows of each revision since the first, oldest first, while the
    /// copy has no remote.
    history: Vec<Rows>,
    /// The remote's `latest`.
    latest: Rows,
    tracked: BTreeSet<String>,
    disk: Rows,
    committed: bool,
    remote: bool,
    /// Pulls that brought a newer revision, and undos that ran, so far.
    moved: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scope {
    #[default]
    Files,
    Package,
    Folder,
}

impl Scope {
    fn sync_scope(self) -> Res<SyncScope> {
        Ok(match self {
            Scope::Files => SyncScope::IndividualFiles,
            Scope::Package => SyncScope::EntirePackage,
            Scope::Folder => SyncScope::folders(["d"])?,
        })
    }

    /// Whether a path the copy does not track is fetched under this scope.
    fn covers(self, key: &str) -> bool {
        match self {
            Scope::Files => false,
            Scope::Package => true,
            Scope::Folder => key.starts_with("d/"),
        }
    }
}

impl Model {
    /// Whether the pull runs. A copy with a commit pending changes nothing,
    /// whether the pull refuses or finds nothing to do.
    fn pull(&mut self) -> Option<bool> {
        if self.committed {
            return None;
        }
        if !self.remote {
            return Some(false);
        }
        let changed: BTreeSet<&String> = self
            .rows
            .keys()
            .chain(self.latest.keys())
            .filter(|key| self.rows.get(*key) != self.latest.get(*key))
            .collect();
        for key in changed {
            if !self.tracked.contains(key) && !self.scope.covers(key) {
                continue;
            }
            if let Some(body) = self.latest.get(key) {
                self.tracked.insert(key.clone());
                self.disk.insert(key.clone(), body.clone());
            } else {
                self.tracked.remove(key);
                self.disk.remove(key);
            }
        }
        if self.rows != self.latest {
            self.moved += 1;
        }
        self.rows.clone_from(&self.latest);
        Some(true)
    }

    /// The rows the scope covers that the copy neither tracks nor has.
    fn keep(&mut self) {
        let backlog: Vec<String> = self
            .rows
            .keys()
            .filter(|key| {
                !self.tracked.contains(*key)
                    && !self.disk.contains_key(*key)
                    && self.scope.covers(key)
            })
            .cloned()
            .collect();
        for key in backlog {
            self.disk.insert(key.clone(), self.rows[&key].clone());
            self.tracked.insert(key);
        }
    }

    fn install(&mut self, key: &str) {
        if !self.tracked.contains(key) && !self.disk.contains_key(key) {
            self.disk.insert(key.to_string(), self.rows[key].clone());
            self.tracked.insert(key.to_string());
        }
    }

    fn uninstall(&mut self, key: &str) {
        self.tracked.remove(key);
        self.disk.remove(key);
    }

    /// A commit of what the user did to `disk`.
    fn commit(&mut self) {
        let gone: Vec<String> = self
            .tracked
            .iter()
            .filter(|key| !self.disk.contains_key(*key))
            .cloned()
            .collect();
        for key in gone {
            self.tracked.remove(&key);
            self.rows.remove(&key);
        }
        for (key, body) in &self.disk {
            self.tracked.insert(key.clone());
            self.rows.insert(key.clone(), body.clone());
        }
        self.committed = true;
        if !self.remote {
            self.history.push(self.rows.clone());
        }
    }

    /// Whether the undo runs; it refuses where a path it would touch does not
    /// hold what the revision being undone recorded there.
    fn undo(&mut self) -> bool {
        if self.remote || self.history.len() < 2 {
            return false;
        }
        let previous = self.history[self.history.len() - 2].clone();
        let restored: BTreeSet<&String> = previous
            .iter()
            .filter(|(key, body)| self.rows.get(*key) != Some(body))
            .map(|(key, _)| key)
            .collect();
        let clean = self
            .tracked
            .iter()
            .chain(restored.iter().copied())
            .all(|key| self.disk.get(key) == self.rows.get(key));
        if !clean {
            return false;
        }
        for key in self.rows.keys().filter(|key| !previous.contains_key(*key)) {
            self.tracked.remove(key);
            self.disk.remove(key);
        }
        for key in restored {
            self.tracked.insert(key.clone());
            self.disk.insert(key.clone(), previous[key].clone());
        }
        self.history.pop();
        self.rows = previous;
        self.moved += 1;
        true
    }
}

/// A copy of `f/a` and the remote it came from.
struct Fixture {
    package: InstalledPackage<LocalStorage, MockRemote>,
    scratch: tempfile::TempDir,
    _root: tempfile::TempDir,
    /// Each row published so far, by its key and bytes.
    published: BTreeMap<(String, Vec<u8>), ManifestRow>,
}

impl Fixture {
    /// Installed from a remote whose only revision holds `rows`.
    async fn installed(rows: &Rows) -> Res<Self> {
        let root = tempfile::TempDir::new()?;
        let paths = DomainPaths::new(root.path().join("domain"));
        let storage = LocalStorage::new();
        let lineage =
            DomainLineageIo::write_fixture(BACKEND, &storage, &paths, root.path().join("home"), [])
                .await?;
        let domain =
            LocalDomain::with_parts(paths, lineage, storage, Arc::new(MockRemote::default()));
        let mut copy = Self {
            package: domain.create_installed_package(("f", "a").into()),
            scratch: tempfile::TempDir::new()?,
            _root: root,
            published: BTreeMap::new(),
        };
        let hash = copy.publish(rows).await?;
        let uri = ManifestUri {
            origin: None,
            bucket: BUCKET.to_string(),
            namespace: ("f", "a").into(),
            hash,
        };
        domain.install_package(&uri).await?;
        Ok(copy)
    }

    /// Created locally from `rows`, with no remote.
    async fn created(rows: &Rows) -> Res<Self> {
        let root = tempfile::TempDir::new()?;
        let source = root.path().join("source");
        for (key, body) in rows {
            let file = source.join(key);
            std::fs::create_dir_all(file.parent().expect("a folder"))?;
            std::fs::write(file, body)?;
        }
        let paths = DomainPaths::new(root.path().join("domain"));
        let storage = LocalStorage::new();
        let lineage =
            DomainLineageIo::write_fixture(BACKEND, &storage, &paths, root.path().join("home"), [])
                .await?;
        let domain =
            LocalDomain::with_parts(paths, lineage, storage, Arc::new(MockRemote::default()));
        let (package, _) = domain
            .create_package(("f", "a").into(), Some(source), None, UserMeta::Clear)
            .await?;
        Ok(Self {
            package,
            scratch: tempfile::TempDir::new()?,
            _root: root,
            published: BTreeMap::new(),
        })
    }

    /// Publishes `rows` as the remote's `latest`, uploading only the rows
    /// not published before.
    async fn publish(&mut self, rows: &Rows) -> Res<String> {
        let remote = &*self.package.remote;
        let mut manifest = Vec::new();
        for (key, body) in rows {
            let published = (key.clone(), body.clone());
            if !self.published.contains_key(&published) {
                let version = format!("v{}", self.published.len());
                let row = publish_row(remote, self.scratch.path(), key, &version, body).await?;
                self.published.insert(published.clone(), row);
            }
            manifest.push(self.published[&published].clone());
        }
        let hash = publish_manifest(remote, manifest).await?;
        let uri = ManifestUri {
            origin: None,
            bucket: BUCKET.to_string(),
            namespace: ("f", "a").into(),
            hash,
        };
        tag_latest(remote, &uri).await?;
        Ok(uri.hash)
    }

    async fn tracked(&self) -> Res<BTreeSet<String>> {
        Ok(self
            .package
            .lineage()
            .await?
            .paths
            .keys()
            .map(|key| key.to_string_lossy().replace('\\', "/"))
            .collect())
    }

    async fn write(&self, key: &str, body: &[u8]) -> Res {
        let file = self.package.package_home().await?.join(key);
        std::fs::create_dir_all(file.parent().expect("a folder"))?;
        Ok(std::fs::write(file, body)?)
    }

    async fn delete(&self, key: &str) -> Res {
        Ok(std::fs::remove_file(
            self.package.package_home().await?.join(key),
        )?)
    }

    async fn commit(&self) -> Res {
        self.package
            .commit(
                "step".to_string(),
                UserMeta::Keep,
                None,
                Some(HostConfig::default()),
            )
            .await?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum Step {
    /// The remote's `latest` moves on: one row changed, added or removed.
    Publish(String, Option<Vec<u8>>),
    Pull,
    /// `keep` under the copy's scope, downloading its backlog.
    Keep,
    Install(String),
    Uninstall(String),
    /// The user writes or deletes a file, then commits.
    Commit(String, Option<Vec<u8>>),
    Undo,
}

/// A step a copy in `model`'s state can take.
/// A remote copy commits only when `late`: a commit pending stops its pulls.
fn draw(model: &Model, rng: &mut Draw, made: &mut u32, late: bool) -> Step {
    let mut body = |key: &str| {
        *made += 1;
        format!("{key}:{made}").into_bytes()
    };
    let keys: Vec<String> = KEYS.iter().map(ToString::to_string).collect();
    let tracked: Vec<String> = model.tracked.iter().cloned().collect();
    let untracked_rows: Vec<String> = model
        .rows
        .keys()
        .filter(|key| !model.tracked.contains(*key))
        .cloned()
        .collect();
    let new: Vec<String> = keys
        .iter()
        .filter(|key| !model.rows.contains_key(*key) && !model.disk.contains_key(*key))
        .cloned()
        .collect();
    let commits = late || !model.remote;
    loop {
        let step = match rng.below(if model.remote { 12 } else { 5 }) {
            1..=3 if !commits => None,
            0 => rng.pick(&tracked).map(|key| Step::Uninstall(key.clone())),
            1 => rng
                .pick(&tracked)
                .map(|key| Step::Commit(key.clone(), Some(body(key)))),
            2 => rng
                .pick(&tracked)
                .map(|key| Step::Commit(key.clone(), None)),
            3 => rng
                .pick(&new)
                .map(|key| Step::Commit(key.clone(), Some(body(key)))),
            4 => Some(Step::Undo),
            5 | 6 => Some(Step::Pull),
            7 => Some(Step::Keep),
            8 => rng
                .pick(&untracked_rows)
                .map(|key| Step::Install(key.clone())),
            _ => {
                let key = rng.pick(&keys).expect("keys").clone();
                let remove =
                    model.latest.contains_key(&key) && model.latest.len() > 1 && rng.below(3) == 0;
                let body = (!remove).then(|| body(&key));
                Some(Step::Publish(key, body))
            }
        };
        if let Some(step) = step {
            return step;
        }
    }
}

/// Takes `step` on `copy` and on `model`, then compares their tracked sets.
async fn take(copy: &mut Fixture, model: &mut Model, step: &Step) -> Res<Option<String>> {
    let package = &copy.package;
    let ran = match step {
        Step::Publish(key, body) => {
            match body {
                Some(body) => model.latest.insert(key.clone(), body.clone()),
                None => model.latest.remove(key),
            };
            let latest = model.latest.clone();
            copy.publish(&latest).await?;
            None
        }
        Step::Pull => {
            let expected = model.pull();
            let pulled = package
                .pull(Some(HostConfig::default()), model.scope.sync_scope()?)
                .await;
            expected
                .is_some_and(|expected| pulled.is_ok() != expected)
                .then(|| format!("pull: {pulled:?}"))
        }
        Step::Keep => {
            model.keep();
            package.keep(model.scope.sync_scope()?, true).await?;
            None
        }
        Step::Install(key) => {
            model.install(key);
            package.install_paths(&[PathBuf::from(key)]).await?;
            None
        }
        Step::Uninstall(key) => {
            model.uninstall(key);
            package.uninstall_paths(&vec![PathBuf::from(key)]).await?;
            None
        }
        Step::Commit(key, body) => {
            if let Some(body) = body {
                model.disk.insert(key.clone(), body.clone());
                copy.write(key, body).await?;
            } else {
                model.disk.remove(key);
                copy.delete(key).await?;
            }
            model.commit();
            copy.commit().await?;
            None
        }
        Step::Undo => {
            let expected = model.undo();
            let undone = package.undo_commit().await;
            (undone.is_ok() != expected).then(|| format!("undo: {undone:?}"))
        }
    };
    let tracked = copy.tracked().await?;
    let rows: BTreeSet<String> = copy
        .package
        .manifest()
        .await?
        .rows
        .iter()
        .map(|row| row.logical_key.to_string())
        .collect();
    let expected_rows: BTreeSet<String> = model.rows.keys().cloned().collect();
    Ok(ran
        .or_else(|| {
            (tracked != model.tracked)
                .then(|| format!("tracks {tracked:?}, the model {:?}", model.tracked))
        })
        .or_else(|| {
            (rows != expected_rows)
                .then(|| format!("has rows {rows:?}, the model {expected_rows:?}"))
        }))
}

/// Runs `steps` random steps from `seed` on a copy under `scope`, remote or
/// local, failing at the first step the model disagrees with. Returns how
/// many pulls or undos moved the copy to another revision.
async fn sequence(seed: u64, scope: Scope, remote: bool, steps: usize) -> Res<usize> {
    let rows: Rows = [("a.txt", "a:0"), ("d/e.txt", "d/e:0")]
        .into_iter()
        .map(|(key, body)| (key.to_string(), body.as_bytes().to_vec()))
        .collect();
    let mut copy = if remote {
        Fixture::installed(&rows).await?
    } else {
        Fixture::created(&rows).await?
    };
    let mut model = Model {
        scope,
        tracked: if remote {
            BTreeSet::new()
        } else {
            rows.keys().cloned().collect()
        },
        disk: if remote { Rows::new() } else { rows.clone() },
        history: if remote {
            Vec::new()
        } else {
            vec![rows.clone()]
        },
        latest: if remote { rows.clone() } else { Rows::new() },
        rows,
        remote,
        ..Model::default()
    };
    copy.package.keep(scope.sync_scope()?, false).await?;

    let mut random = Draw(seed);
    let mut made = 0;
    let mut taken = Vec::new();
    for taking in 0..steps {
        let step = draw(&model, &mut random, &mut made, taking >= steps * 2 / 3);
        taken.push(step.clone());
        let differs = match take(&mut copy, &mut model, &step).await {
            Ok(differs) => differs,
            Err(err) => Some(format!("{err}")),
        };
        if let Some(differs) = differs {
            panic!("seed {seed}, {scope:?}, remote {remote}: {differs} after {taken:#?}");
        }
    }
    Ok(model.moved)
}

/// Eight sequences of a remote copy under `scope`.
async fn remote_sequences(scope: Scope) -> Res {
    let mut pulled = 0;
    for seed in 1..=8 {
        pulled += sequence(seed * 7919, scope, true, 14).await?;
    }
    assert!(
        pulled >= 8,
        "{scope:?}: only {pulled} pulls brought a revision"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_remote_copy_of_individual_files_tracks_what_the_model_predicts() -> Res {
    remote_sequences(Scope::Files).await
}

#[test(tokio::test)]
async fn a_remote_copy_of_the_entire_package_tracks_what_the_model_predicts() -> Res {
    remote_sequences(Scope::Package).await
}

#[test(tokio::test)]
async fn a_remote_copy_of_a_folder_tracks_what_the_model_predicts() -> Res {
    remote_sequences(Scope::Folder).await
}

#[test(tokio::test)]
async fn a_local_copy_tracks_what_the_model_predicts_through_undo() -> Res {
    let mut undone = 0;
    for seed in 1..=12 {
        undone += sequence(seed * 104_729, Scope::Files, false, 14).await?;
    }
    assert!(undone >= 8, "only {undone} undos ran");
    Ok(())
}

/// The sequence the spec's evidence ran under the entire-package scope,
/// with the keys it recorded at each step.
#[test(tokio::test)]
async fn the_evidence_sequence_tracks_as_recorded() -> Res {
    let key = |key: &str| key.to_string();
    let a = Rows::from([(key("a.txt"), b"a".to_vec())]);
    let mut copy = Fixture::installed(&a).await?;
    let keys = |keys: &[&str]| {
        keys.iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>()
    };
    let package = copy.package.clone();

    package.keep(SyncScope::EntirePackage, false).await?;
    assert_eq!(
        copy.tracked().await?,
        keys(&[]),
        "scope set, nothing pulled"
    );

    let mut ab = a.clone();
    ab.insert(key("b.txt"), b"b".to_vec());
    copy.publish(&ab).await?;
    package
        .pull(Some(HostConfig::default()), SyncScope::EntirePackage)
        .await?;
    assert_eq!(copy.tracked().await?, keys(&["b.txt"]), "pulled");

    package.keep(SyncScope::EntirePackage, true).await?;
    assert_eq!(copy.tracked().await?, keys(&["a.txt", "b.txt"]), "kept");

    package
        .uninstall_paths(&vec![PathBuf::from("a.txt")])
        .await?;
    assert_eq!(copy.tracked().await?, keys(&["b.txt"]), "a uninstalled");

    copy.delete("b.txt").await?;
    assert_eq!(copy.tracked().await?, keys(&["b.txt"]), "b deleted");
    copy.commit().await?;
    assert_eq!(copy.tracked().await?, keys(&[]), "the deletion committed");

    copy.write("c.txt", b"c").await?;
    copy.commit().await?;
    assert_eq!(copy.tracked().await?, keys(&["c.txt"]), "c committed");
    Ok(())
}
