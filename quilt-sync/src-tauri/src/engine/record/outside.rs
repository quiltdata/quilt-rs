//! Changes another tool made on this computer: a package's revision moved,
//! and no write of this process moved it.
//!
//! Every lineage write this process makes reaches [`Outside::hook`] as it
//! lands, and the revision it leaves is remembered. A pass,
//! [`Outside::check`], reads every package under the lock its writes take
//! and records each revision that moved since: only another process, such as
//! the quilt CLI, writes without the hook. A write here first checks the entry it
//! replaces, so a move made just before it is not lost.
//!
//! `outside.json` in the data folder keeps what was seen, so a move made
//! while `QuiltSync` was closed is found on the first pass after start.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use quilt_rs::io::storage::LocalStorage;
use quilt_rs::lineage::DomainLineageIo;
use quilt_rs::lineage::LineageHook;
use quilt_rs::lineage::PackageLineage;
use quilt_uri::Namespace;
use serde::Deserialize;
use serde::Serialize;

use super::Draft;
use super::Record;
use crate::engine::telemetry::prelude::*;

const FILE_NAME: &str = "outside.json";

/// What a package's entry said when last seen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Seen {
    /// The revision the copy is on in its bucket.
    remote: Option<String>,
    /// The copy's own commit, while it is not that revision.
    unpublished: Option<String>,
}

impl Seen {
    fn of(lineage: &PackageLineage) -> Self {
        let remote = lineage
            .remote_uri
            .as_ref()
            .map(|r| r.hash.clone())
            .filter(|h| !h.is_empty());
        let unpublished = lineage
            .commit
            .as_ref()
            .map(|c| c.hash.clone())
            .filter(|h| !h.is_empty() && Some(h) != remote.as_ref());
        Self {
            remote,
            unpublished,
        }
    }
}

type SeenMap = BTreeMap<Namespace, Seen>;

struct State {
    seen: SeenMap,
    /// What the file holds, as last read or written.
    saved: SeenMap,
}

/// Remember `now` as `namespace`'s entry, `None` when it is not installed,
/// and return the entry for a move since it was last seen. A package seen
/// for the first time has no move.
fn saw(seen: &mut SeenMap, namespace: &Namespace, now: Option<&PackageLineage>) -> Option<Draft> {
    let Some(now) = now.map(Seen::of) else {
        seen.remove(namespace);
        return None;
    };
    let was = seen.insert(namespace.clone(), now.clone())?;
    if was.remote == now.remote {
        return None;
    }
    let to = now.remote?;
    let published = was.unpublished.as_deref() == Some(to.as_str());
    Some(Draft::outside(
        namespace,
        was.remote.as_deref(),
        &to,
        published,
    ))
}

pub struct Outside {
    /// `None` keeps what was seen in memory only.
    path: Option<PathBuf>,
    lineage: DomainLineageIo,
    state: Mutex<State>,
    record: Arc<Record>,
}

impl Outside {
    /// What was seen in `data_dir`, for the domain whose lineage `lineage`
    /// reads. A file that cannot be read starts empty: the next pass
    /// remembers every package and records nothing.
    pub async fn open(data_dir: &Path, lineage: DomainLineageIo, record: Arc<Record>) -> Arc<Self> {
        let path = data_dir.join(FILE_NAME);
        let seen = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                warn!(
                    "outside: {} does not parse, starting over: {err}",
                    path.display()
                );
                SeenMap::new()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => SeenMap::new(),
            Err(err) => {
                warn!(
                    "outside: could not read {}, starting over: {err}",
                    path.display()
                );
                SeenMap::new()
            }
        };
        Arc::new(Self {
            path: Some(path),
            lineage,
            state: Mutex::new(State {
                saved: seen.clone(),
                seen,
            }),
            record,
        })
    }

    #[cfg(test)]
    fn in_memory(lineage: DomainLineageIo, record: Arc<Record>) -> Arc<Self> {
        Arc::new(Self {
            path: None,
            lineage,
            state: Mutex::new(State {
                seen: SeenMap::new(),
                saved: SeenMap::new(),
            }),
            record,
        })
    }

    /// The hook for this process's domain: each write remembers the entry it
    /// leaves, after recording a move made to the entry it replaces.
    pub fn hook(self: &Arc<Self>) -> LineageHook {
        let outside = Arc::clone(self);
        LineageHook::new(move |namespace, was, is| {
            let mut state = outside.state.lock().expect("outside lock");
            if let Some(draft) = saw(&mut state.seen, namespace, was) {
                outside.record.insert_later(outside.record.number(draft));
            }
            saw(&mut state.seen, namespace, is);
            outside.save(&mut state);
        })
    }

    /// Record every package whose revision moved since it was last seen.
    pub async fn check(&self) {
        let found = self
            .lineage
            .check_each(
                &LocalStorage::new(),
                |namespace, lineage| {
                    let mut state = self.state.lock().expect("outside lock");
                    // Numbered under the lock `check_each` holds, so a write
                    // after it, ours or another tool's, comes later; kept as
                    // pending, so a stop waits for it.
                    if let Some(draft) = saw(&mut state.seen, namespace, Some(lineage)) {
                        self.record.insert_later(self.record.number(draft));
                        // Saved with it: a stop before `finish` must not
                        // leave the move to be recorded again.
                        self.save(&mut state);
                    }
                },
                |installed| {
                    let mut state = self.state.lock().expect("outside lock");
                    // Unknown while a package is installed or removed: a
                    // later pass forgets what is gone.
                    if let Some(installed) = installed {
                        state
                            .seen
                            .retain(|namespace, _| installed.contains(namespace));
                    }
                    self.save(&mut state);
                },
            )
            .await;
        if let Err(err) = found {
            debug!("outside: could not read the packages: {err}");
        }
    }

    /// Write what was seen when the file holds something else. The hook calls
    /// it under the lock its write holds, so it is written at once: a stop
    /// right after a write must not leave an older revision here, which the
    /// next start would take for another tool's move.
    fn save(&self, state: &mut State) {
        let Some(path) = &self.path else {
            return;
        };
        if state.saved == state.seen {
            return;
        }
        let result = serde_json::to_vec(&state.seen)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                let partial = path.with_extension("json.partial");
                std::fs::write(&partial, bytes)?;
                std::fs::rename(&partial, path)
            });
        match result {
            Ok(()) => state.saved = state.seen.clone(),
            Err(err) => warn!("outside: could not write {}: {err}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::LocalDomain;
    use quilt_rs::lineage::CommitState;
    use quilt_rs::lineage::LineageStore;
    use quilt_rs::paths::DomainPaths;
    use quilt_uri::ManifestUri;

    use crate::engine::events::EventSink;
    use crate::engine::record::Kind;
    use crate::engine::record::Revision;

    fn ns() -> Namespace {
        Namespace::from(("foo", "bar"))
    }

    /// A hex letter stands for the top hash made of 64 of it.
    fn top_hash(letter: &str) -> String {
        letter.repeat(64)
    }

    fn on(remote: &str, commit: Option<&str>) -> PackageLineage {
        PackageLineage {
            remote_uri: Some(ManifestUri {
                bucket: "bucket".to_string(),
                namespace: ns(),
                hash: top_hash(remote),
                origin: None,
            }),
            commit: commit.map(|hash| CommitState {
                hash: top_hash(hash),
                ..CommitState::default()
            }),
            ..PackageLineage::default()
        }
    }

    #[test]
    fn a_first_sight_is_no_move() {
        let mut seen = SeenMap::new();
        assert_eq!(saw(&mut seen, &ns(), Some(&on("a", None))), None);
        assert_eq!(saw(&mut seen, &ns(), Some(&on("a", Some("c")))), None);
    }

    #[test]
    fn a_moved_revision_is_updated() {
        let mut seen = SeenMap::new();
        saw(&mut seen, &ns(), Some(&on("a", None)));
        let draft = saw(&mut seen, &ns(), Some(&on("b", None))).unwrap();
        assert_eq!(draft.kind, Kind::Outside { published: false });
        assert_eq!(
            draft.revision,
            Some(Revision {
                from: Some(top_hash("a")),
                to: top_hash("b")
            })
        );
        assert_eq!(draft.files, vec![]);
    }

    #[test]
    fn the_copys_own_commit_is_published() {
        let mut seen = SeenMap::new();
        saw(&mut seen, &ns(), Some(&on("a", Some("c"))));
        let draft = saw(&mut seen, &ns(), Some(&on("c", Some("c")))).unwrap();
        assert_eq!(draft.kind, Kind::Outside { published: true });
    }

    #[test]
    fn an_uninstalled_package_is_forgotten() {
        let mut seen = SeenMap::new();
        saw(&mut seen, &ns(), Some(&on("a", None)));
        assert_eq!(saw(&mut seen, &ns(), None), None);
        assert_eq!(saw(&mut seen, &ns(), Some(&on("b", None))), None);
    }

    /// Runs each named case once on each lineage store.
    macro_rules! on_each_store {
        ($($case:ident),* $(,)?) => {
            mod on_data_json {
                $(#[tokio::test]
                async fn $case() {
                    super::$case(super::LineageStore::DataJson).await;
                })*
            }
            mod on_refs {
                $(#[tokio::test]
                async fn $case() {
                    super::$case(super::LineageStore::Refs).await;
                })*
            }
        };
    }

    on_each_store!(
        our_own_writes_are_not_outside,
        another_tools_write_is_found_by_the_pass,
        our_write_after_another_tools_finds_its_move,
        a_busy_package_keeps_what_was_seen,
        an_unreadable_package_keeps_what_was_seen,
        a_failed_save_is_tried_again,
        a_move_while_closed_is_found_after_start,
    );

    /// The lineage of a domain on `dir` in `store`, with its home set.
    async fn domain(dir: &Path, store: LineageStore) -> DomainLineageIo {
        let domain = LocalDomain::new(dir).with_lineage_store(store);
        domain.set_home(dir).await.unwrap();
        domain.lineage_io().clone()
    }

    /// A domain on `dir` whose writes go through `outside`'s hook, as this
    /// process's do, and one without it, as the CLI's.
    async fn setup(
        store: LineageStore,
    ) -> (
        tempfile::TempDir,
        Arc<Record>,
        Arc<Outside>,
        DomainLineageIo,
        DomainLineageIo,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let theirs = domain(dir.path(), store).await;
        let record = Arc::new(Record::in_memory(EventSink::new()));
        let outside = Outside::in_memory(theirs.clone(), Arc::clone(&record));
        let ours = theirs.clone().with_hook(outside.hook());
        (dir, record, outside, ours, theirs)
    }

    async fn put(io: &DomainLineageIo, lineage: PackageLineage) {
        io.update_package_lineage(&LocalStorage::new(), &ns(), |entry| {
            *entry = Some(lineage);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Entries inserted in the background have landed.
    async fn kinds(record: &Record) -> Vec<Kind> {
        record.settle().await;
        record.entries().await.into_iter().map(|e| e.kind).collect()
    }

    async fn our_own_writes_are_not_outside(store: LineageStore) {
        let (_dir, record, outside, ours, _) = setup(store).await;
        put(&ours, on("a", None)).await;
        outside.check().await;
        put(&ours, on("b", None)).await;
        outside.check().await;
        assert_eq!(kinds(&record).await, vec![]);
    }

    async fn another_tools_write_is_found_by_the_pass(store: LineageStore) {
        let (_dir, record, outside, ours, theirs) = setup(store).await;
        put(&ours, on("a", Some("c"))).await;
        put(&theirs, on("c", None)).await;
        outside.check().await;
        outside.check().await;
        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: true }]
        );
    }

    /// Our write right after another tool's, before any pass, still records
    /// the other tool's move.
    async fn our_write_after_another_tools_finds_its_move(store: LineageStore) {
        let (_dir, record, outside, ours, theirs) = setup(store).await;
        put(&ours, on("a", None)).await;
        put(&theirs, on("b", None)).await;
        put(&ours, on("c", None)).await;
        outside.check().await;
        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }

    /// A package the pass finds locked keeps what was seen, so a move made
    /// before it is found once the lock is free.
    async fn a_busy_package_keeps_what_was_seen(store: LineageStore) {
        let (dir, record, outside, _, theirs) = setup(store).await;
        put(&theirs, on("a", None)).await;
        outside.check().await;
        put(&theirs, on("b", None)).await;

        let package = LocalDomain::new(dir.path())
            .with_lineage_store(store)
            .create_installed_package(ns());
        let held = package.lock().await.unwrap();
        outside.check().await;
        drop(held);
        outside.check().await;

        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }

    /// A package whose entry can't be read keeps what was seen, so a move is
    /// found once it reads again.
    async fn an_unreadable_package_keeps_what_was_seen(store: LineageStore) {
        let (dir, record, outside, _, theirs) = setup(store).await;
        let storage = LocalStorage::new();
        put(&theirs, on("a", None)).await;
        outside.check().await;

        theirs.damage_fixture(&storage, &ns()).await.unwrap();
        outside.check().await;
        let paths = DomainPaths::new(dir.path().to_path_buf());
        DomainLineageIo::write_fixture(
            store,
            &storage,
            &paths,
            dir.path(),
            [(ns(), on("b", None))],
        )
        .await
        .unwrap();
        outside.check().await;

        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }

    /// A pass that can't tell what is installed forgets nothing, so a package
    /// another tool uninstalls and installs again at another revision is
    /// found to have moved.
    #[tokio::test]
    async fn a_pass_that_cant_list_the_packages_forgets_nothing() {
        let (dir, record, outside, _, theirs) = setup(LineageStore::Refs).await;
        put(&theirs, on("a", None)).await;
        outside.check().await;
        theirs
            .update_package_lineage(&LocalStorage::new(), &ns(), |entry| {
                *entry = None;
                Ok(())
            })
            .await
            .unwrap();

        // The ref store's domain lock, as an install holds it.
        let domain_lock = DomainPaths::new(dir.path().to_path_buf())
            .dot_quilt_dir()
            .join("domain.lock");
        let held = std::fs::File::create(domain_lock).unwrap();
        held.lock().unwrap();
        outside.check().await;
        drop(held);
        put(&theirs, on("b", None)).await;
        outside.check().await;

        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }

    /// A write that failed is tried again on the next pass, though nothing
    /// changed since.
    async fn a_failed_save_is_tried_again(store: LineageStore) {
        let dir = tempfile::tempdir().unwrap();
        let theirs = domain(dir.path(), store).await;
        put(&theirs, on("a", None)).await;
        let data_dir = dir.path().join("app");
        let record = Arc::new(Record::in_memory(EventSink::new()));
        let outside = Outside::open(&data_dir, theirs, record).await;

        // The folder is missing, so the write fails.
        outside.check().await;
        assert!(!data_dir.join(FILE_NAME).exists());
        std::fs::create_dir(&data_dir).unwrap();
        outside.check().await;
        assert!(data_dir.join(FILE_NAME).exists());
    }

    /// What was seen survives a restart, so a move while closed is found.
    async fn a_move_while_closed_is_found_after_start(store: LineageStore) {
        let dir = tempfile::tempdir().unwrap();
        let record = Arc::new(Record::in_memory(EventSink::new()));
        let theirs = domain(dir.path(), store).await;
        put(&theirs, on("a", None)).await;

        let first = Outside::open(dir.path(), theirs.clone(), Arc::clone(&record)).await;
        first.check().await;
        drop(first);
        put(&theirs, on("b", None)).await;
        let second = Outside::open(dir.path(), theirs.clone(), Arc::clone(&record)).await;
        second.check().await;

        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }
}
