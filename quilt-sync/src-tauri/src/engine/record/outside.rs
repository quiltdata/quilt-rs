//! Changes another tool made on this computer: a package's revision moved,
//! and no write of this process moved it.
//!
//! Every write of `data.json` this process makes reaches [`Outside::hook`]
//! as it lands, and the revision it leaves is remembered. A pass,
//! [`Outside::check`], reads every package under the same lock and records
//! each revision that moved since: only another process, such as the quilt
//! CLI, writes without the hook. A write here first checks the entry it
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
    seen: Mutex<SeenMap>,
    record: Arc<Record>,
}

impl Outside {
    /// What was seen in `data_dir`, for the domain whose `data.json` is
    /// `lineage`. A file that cannot be read starts empty: the next pass
    /// remembers every package and records nothing.
    pub async fn open(data_dir: &Path, lineage: PathBuf, record: Arc<Record>) -> Arc<Self> {
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
            lineage: DomainLineageIo::new(lineage),
            seen: Mutex::new(seen),
            record,
        })
    }

    #[cfg(test)]
    fn in_memory(lineage: PathBuf, record: Arc<Record>) -> Arc<Self> {
        Arc::new(Self {
            path: None,
            lineage: DomainLineageIo::new(lineage),
            seen: Mutex::new(SeenMap::new()),
            record,
        })
    }

    /// The hook for this process's domain: each write remembers the entry it
    /// leaves, after recording a move made to the entry it replaces.
    pub fn hook(self: &Arc<Self>) -> LineageHook {
        let outside = Arc::clone(self);
        LineageHook::new(move |namespace, was, is| {
            let mut seen = outside.seen.lock().expect("outside lock");
            let before = seen.clone();
            if let Some(draft) = saw(&mut seen, namespace, was) {
                outside.record.insert_later(outside.record.number(draft));
            }
            saw(&mut seen, namespace, is);
            outside.save_if_changed(&before, &seen);
        })
    }

    /// Record every package whose revision moved since it was last seen.
    pub async fn check(&self) {
        let found = self
            .lineage
            .read_locked(&LocalStorage::new(), |domain| {
                let mut seen = self.seen.lock().expect("outside lock");
                let before = seen.clone();
                let drafts: Vec<Draft> = domain
                    .packages
                    .iter()
                    .filter_map(|(namespace, lineage)| saw(&mut seen, namespace, Some(lineage)))
                    .collect();
                seen.retain(|namespace, _| domain.packages.contains_key(namespace));
                self.save_if_changed(&before, &seen);
                drafts
            })
            .await;
        match found {
            Ok(drafts) => {
                for draft in drafts {
                    self.record.add(draft).await;
                }
            }
            Err(err) => debug!("outside: could not read the packages: {err}"),
        }
    }

    /// Write `seen` when it differs from `before`. Called under the lock on
    /// `data.json`, so it is written at once: a stop right after a write
    /// must not leave an older revision here, which the next start would
    /// take for another tool's move.
    fn save_if_changed(&self, before: &SeenMap, seen: &SeenMap) {
        let Some(path) = &self.path else {
            return;
        };
        if before == seen {
            return;
        }
        let result = serde_json::to_vec(seen)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                let partial = path.with_extension("json.partial");
                std::fs::write(&partial, bytes)?;
                std::fs::rename(&partial, path)
            });
        if let Err(err) = result {
            warn!("outside: could not write {}: {err}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::lineage::CommitState;
    use quilt_rs::lineage::DomainLineage;
    use quilt_uri::ManifestUri;

    use crate::engine::events::EventSink;
    use crate::engine::record::Kind;
    use crate::engine::record::Revision;

    fn ns() -> Namespace {
        Namespace::from(("foo", "bar"))
    }

    fn on(remote: &str, commit: Option<&str>) -> PackageLineage {
        PackageLineage {
            remote_uri: Some(ManifestUri {
                bucket: "bucket".to_string(),
                namespace: ns(),
                hash: remote.to_string(),
                origin: None,
            }),
            commit: commit.map(|hash| CommitState {
                hash: hash.to_string(),
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
                from: Some("a".to_string()),
                to: "b".to_string()
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

    /// A domain on `dir` whose writes go through `outside`'s hook, as this
    /// process's do, and one without it, as the CLI's.
    async fn setup() -> (
        tempfile::TempDir,
        Arc<Record>,
        Arc<Outside>,
        DomainLineageIo,
        DomainLineageIo,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        let record = Arc::new(Record::in_memory(EventSink::new()));
        let outside = Outside::in_memory(path.clone(), Arc::clone(&record));
        let ours = DomainLineageIo::new(path.clone()).with_hook(outside.hook());
        let theirs = DomainLineageIo::new(path);
        ours.write(&LocalStorage::new(), DomainLineage::new(dir.path()))
            .await
            .unwrap();
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

    #[tokio::test]
    async fn our_own_writes_are_not_outside() {
        let (_dir, record, outside, ours, _) = setup().await;
        put(&ours, on("a", None)).await;
        outside.check().await;
        put(&ours, on("b", None)).await;
        outside.check().await;
        assert_eq!(kinds(&record).await, vec![]);
    }

    #[tokio::test]
    async fn another_tools_write_is_found_by_the_pass() {
        let (_dir, record, outside, ours, theirs) = setup().await;
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
    #[tokio::test]
    async fn our_write_after_another_tools_finds_its_move() {
        let (_dir, record, outside, ours, theirs) = setup().await;
        put(&ours, on("a", None)).await;
        put(&theirs, on("b", None)).await;
        put(&ours, on("c", None)).await;
        outside.check().await;
        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }

    /// What was seen survives a restart, so a move while closed is found.
    #[tokio::test]
    async fn a_move_while_closed_is_found_after_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        let record = Arc::new(Record::in_memory(EventSink::new()));
        let theirs = DomainLineageIo::new(path.clone());
        let storage = LocalStorage::new();
        theirs
            .write(&storage, DomainLineage::new(dir.path()))
            .await
            .unwrap();
        put(&theirs, on("a", None)).await;

        let first = Outside::open(dir.path(), path.clone(), Arc::clone(&record)).await;
        first.check().await;
        drop(first);
        put(&theirs, on("b", None)).await;
        let second = Outside::open(dir.path(), path, Arc::clone(&record)).await;
        second.check().await;

        assert_eq!(
            kinds(&record).await,
            vec![Kind::Outside { published: false }]
        );
    }
}
