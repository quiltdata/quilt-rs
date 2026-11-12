//! The record of what happened: one entry per event, newest last, never
//! merged. Ten pulls are ten entries, because each report describes a span
//! the next one does not extend.
//!
//! The window's inbox lists it, and an entry is read once its panel or its
//! package's page has stayed open for a while. Action-needed states are not
//! written here: they live in [`action_needed`](crate::engine::action_needed)
//! while they hold.
//!
//! # The file
//!
//! `record.jsonl` in the data folder, one line per change: an entry added, or
//! a run of entries marked read. Lines are appended, so a pull or a read never
//! rewrites the file; it is rewritten whole only at start and when the lines
//! outgrow the entries they hold. A line that does not parse is skipped.

use std::collections::VecDeque;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use quilt_rs::flow::PullReport;
use quilt_rs::lineage::Change;
use quilt_rs::lineage::ChangeSet;
use quilt_uri::Namespace;
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::engine::events::EventSink;
use crate::engine::telemetry::prelude::*;

/// Event the window refetches the record on. Kept in lockstep with the UI.
pub const RECORD_EVENT: &str = "record-changed";

/// How long an entry is kept, read or not.
const KEEP_FOR: chrono::TimeDelta = chrono::TimeDelta::days(90);

/// How many entries are kept, dropping the oldest.
const MAX_ENTRIES: usize = 1_000;

/// How many paths one entry keeps. Its counts stay whole.
const MAX_PATHS: usize = 200;

const FILE_NAME: &str = "record.jsonl";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Kind {
    /// A pull brought a newer revision.
    Got,
    Published,
    Uninstalled,
    Created,
    RemovedRevisions {
        count: usize,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileChange {
    New,
    NewNotDownloaded,
    Updated,
    Removed,
    /// Removed by the remote, but edited here, so kept and no longer tracked.
    Kept,
    /// Kept out of a created package by ignore rules.
    LeftOut,
}

/// The files of one kind of change. `count` is whole; `paths` may hold fewer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileGroup {
    pub change: FileChange,
    pub count: usize,
    pub paths: Vec<String>,
}

impl FileGroup {
    /// `2 files new`.
    fn heading(&self) -> String {
        let files = if self.count == 1 { "file" } else { "files" };
        let change = match self.change {
            FileChange::New => "new",
            FileChange::NewNotDownloaded => "new, not downloaded",
            FileChange::Updated => "updated",
            FileChange::Removed => "removed",
            FileChange::Kept => "kept, removed on the remote",
            FileChange::LeftOut => "left out by ignore rules",
        };
        format!("{} {files} {change}", self.count)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub from: Option<String>,
    pub to: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    #[serde(flatten)]
    pub kind: Kind,
    pub namespace: Namespace,
    pub revision: Option<Revision>,
    pub files: Vec<FileGroup>,
    /// What the event reports beyond its files, as a sentence: space freed,
    /// files kept for a busy package, a prune that failed.
    pub outcome: Option<String>,
    pub at: DateTime<Utc>,
    pub read: bool,
}

/// An entry before the record numbers and dates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub kind: Kind,
    pub namespace: Namespace,
    pub revision: Option<Revision>,
    pub files: Vec<FileGroup>,
    pub outcome: Option<String>,
}

impl Draft {
    fn new(kind: Kind, namespace: Namespace) -> Self {
        Self {
            kind,
            namespace,
            revision: None,
            files: Vec::new(),
            outcome: None,
        }
    }

    /// A pull, from the report the engine returned. `from` is the revision
    /// the copy was on.
    #[must_use]
    pub fn got(namespace: &Namespace, from: Option<&str>, report: &PullReport) -> Self {
        let groups = [
            (FileChange::New, &report.added),
            (FileChange::NewNotDownloaded, &report.added_not_fetched),
            (FileChange::Updated, &report.updated),
            (FileChange::Removed, &report.removed),
            (FileChange::Kept, &report.kept_not_removed),
        ]
        .map(|(change, paths)| (change, paths.iter().map(|p| p.display().to_string())));
        Self {
            revision: Some(revision(from, &report.manifest_uri.hash)),
            files: groups_of(groups),
            ..Self::new(Kind::Got, namespace.clone())
        }
    }

    /// A publish of `to`, committing `changes`: empty for a commit made
    /// before.
    #[must_use]
    pub fn published(
        namespace: &Namespace,
        from: Option<&str>,
        to: &str,
        changes: &ChangeSet,
    ) -> Self {
        let of = |wanted: fn(&Change) -> bool| {
            changes
                .iter()
                .filter(move |(_, change)| wanted(change))
                .map(|(path, _)| path.display().to_string())
        };
        let groups = [
            (FileChange::New, of(|c| matches!(c, Change::Added(_)))),
            (
                FileChange::Updated,
                of(|c| matches!(c, Change::Modified(_))),
            ),
            (FileChange::Removed, of(|c| matches!(c, Change::Removed(_)))),
        ];
        Self {
            revision: Some(revision(from, to)),
            files: groups_of(groups),
            ..Self::new(Kind::Published, namespace.clone())
        }
    }

    #[must_use]
    pub fn uninstalled(namespace: &Namespace, outcome: Option<String>) -> Self {
        Self {
            outcome,
            ..Self::new(Kind::Uninstalled, namespace.clone())
        }
    }

    /// A package created, with what the ignore rules left out. A folder left
    /// out is named with a trailing `/`.
    #[must_use]
    pub fn created(namespace: &Namespace, left_out: &quilt_rs::flow::LeftOut) -> Self {
        let files = left_out
            .files
            .iter()
            .map(|(path, _)| path.display().to_string());
        let dirs = left_out
            .dirs
            .iter()
            .map(|(path, _)| format!("{}/", path.display()));
        Self {
            files: groups_of([(FileChange::LeftOut, files.chain(dirs))]),
            ..Self::new(Kind::Created, namespace.clone())
        }
    }

    #[must_use]
    pub fn removed_revisions(report: &quilt_rs::flow::RemovalReport) -> Self {
        let outcome = match &report.kept_for {
            Some(busy) => format!(
                "Kept {} while {busy} is busy.",
                quilt_rs::flow::format_bytes(report.kept_bytes)
            ),
            None if report.bytes == 0 => "Freed no space.".to_string(),
            None => format!("Freed {}.", quilt_rs::flow::format_bytes(report.bytes)),
        };
        Self {
            outcome: Some(outcome),
            ..Self::new(
                Kind::RemovedRevisions {
                    count: report.revisions,
                },
                report.namespace.clone(),
            )
        }
    }

    /// A publish, saying so when its revision did not become `latest`
    /// because another writer moved it first.
    #[must_use]
    pub fn latest_or_not(self, certified: bool) -> Self {
        if certified {
            return self;
        }
        Self {
            outcome: Some(
                "The bucket has newer changes, so this revision is not the latest.".to_string(),
            ),
            ..self
        }
    }

    /// The file groups in a line: `2 files new, 1 file updated`.
    #[cfg(test)]
    #[must_use]
    pub fn files_line(&self) -> String {
        self.files
            .iter()
            .map(FileGroup::heading)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The file groups with the first `named` paths of each:
    /// `2 files new: a.csv, b.csv; 1 file updated: c.csv`.
    #[must_use]
    pub fn files_named(&self, named: usize) -> String {
        self.files
            .iter()
            .map(|group| {
                let shown: Vec<&str> = group.paths.iter().take(named).map(String::as_str).collect();
                let more = group.count.saturating_sub(shown.len());
                let paths = shown.join(", ");
                let more = if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                };
                format!("{}: {paths}{more}", group.heading())
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// What `after` changes from `before`, by logical key: a revision's change,
/// for one whose publish reports no walk of its own.
#[must_use]
pub fn changes_between(
    before: &[quilt_rs::manifest::ManifestRow],
    after: &[quilt_rs::manifest::ManifestRow],
) -> ChangeSet {
    let old: std::collections::BTreeMap<_, _> =
        before.iter().map(|row| (&row.logical_key, row)).collect();
    let mut changes = ChangeSet::new();
    for row in after {
        match old.get(&row.logical_key) {
            None => {
                changes.insert(row.logical_key.clone(), Change::Added(row.clone()));
            }
            Some(was) if was.hash != row.hash => {
                changes.insert(row.logical_key.clone(), Change::Modified(row.clone()));
            }
            Some(_) => {}
        }
    }
    let kept: std::collections::BTreeSet<_> = after.iter().map(|row| &row.logical_key).collect();
    for row in before {
        if !kept.contains(&row.logical_key) {
            changes.insert(row.logical_key.clone(), Change::Removed(row.clone()));
        }
    }
    changes
}

fn revision(from: Option<&str>, to: &str) -> Revision {
    Revision {
        from: from.filter(|f| !f.is_empty()).map(str::to_owned),
        to: to.to_owned(),
    }
}

/// The groups that hold any files, keeping at most [`MAX_PATHS`] paths across
/// them all, in order.
fn groups_of<P: Iterator<Item = String>>(
    groups: impl IntoIterator<Item = (FileChange, P)>,
) -> Vec<FileGroup> {
    let mut room = MAX_PATHS;
    groups
        .into_iter()
        .filter_map(|(change, paths)| {
            let mut count = 0;
            let mut kept = Vec::new();
            for path in paths {
                count += 1;
                if kept.len() < room {
                    kept.push(path);
                }
            }
            room -= kept.len();
            (count > 0).then_some(FileGroup {
                change,
                count,
                paths: kept,
            })
        })
        .collect()
}

/// One line of the file.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Line {
    Add(Entry),
    Read(ReadMark),
}

/// Every entry up to `up_to`, of `namespace` when one is given, is read.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ReadMark {
    up_to: u64,
    namespace: Option<Namespace>,
}

impl ReadMark {
    fn covers(&self, entry: &Entry) -> bool {
        entry.id <= self.up_to
            && self
                .namespace
                .as_ref()
                .is_none_or(|ns| *ns == entry.namespace)
    }
}

struct State {
    /// In id order.
    entries: VecDeque<Entry>,
    /// Lines in the file now; it is rewritten once these outgrow the entries.
    lines: usize,
}

impl State {
    fn apply(&mut self, line: Line) {
        match line {
            Line::Add(entry) => self.insert(entry),
            Line::Read(mark) => {
                for entry in self.entries.iter_mut().filter(|e| mark.covers(e)) {
                    entry.read = true;
                }
            }
        }
    }

    /// In id order: entries numbered in order may arrive out of it.
    fn insert(&mut self, entry: Entry) {
        let at = self.entries.partition_point(|e| e.id < entry.id);
        self.entries.insert(at, entry);
    }

    /// Drop what is past the bounds, oldest first.
    fn trim(&mut self, now: DateTime<Utc>) {
        let oldest = now - KEEP_FOR;
        while self
            .entries
            .front()
            .is_some_and(|e| e.at < oldest || self.entries.len() > MAX_ENTRIES)
        {
            self.entries.pop_front();
        }
    }
}

pub struct Record {
    /// `None` keeps the record in memory only.
    path: Option<PathBuf>,
    state: Mutex<State>,
    /// Handed out by [`Self::number`], so ids follow the order of events
    /// even when their writes do not.
    next_id: AtomicU64,
    /// Writes [`Self::insert_later`] has started and not finished.
    pending: AtomicUsize,
    events: EventSink,
}

/// Where ids start: past every kept one, and never below the time in
/// milliseconds, so a record emptied by age does not hand out ids a window
/// still holds.
fn first_id(last: Option<u64>) -> u64 {
    let now = u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0);
    last.map_or(now, |id| (id + 1).max(now))
}

impl Record {
    /// The record kept in `data_dir`, read and trimmed now.
    pub async fn open(data_dir: &std::path::Path, events: EventSink) -> Self {
        let path = data_dir.join(FILE_NAME);
        let mut state = State {
            entries: VecDeque::new(),
            lines: 0,
        };
        // Line by line, so a line cut short, even inside a character, costs
        // that line alone.
        let read = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                for line in bytes.split(|b| *b == b'\n') {
                    if line.trim_ascii().is_empty() {
                        continue;
                    }
                    match serde_json::from_slice(line) {
                        Ok(line) => state.apply(line),
                        Err(err) => warn!("record: skipped a line that does not parse: {err}"),
                    }
                }
                true
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
            Err(err) => {
                // Kept in memory this run: writing on would number entries
                // over the ones the file holds.
                warn!(
                    "record: could not read {}, keeping this run's record in memory: {err}",
                    path.display()
                );
                false
            }
        };
        state.trim(Utc::now());
        let next_id = first_id(state.entries.back().map(|e| e.id));
        let record = Self {
            path: read.then_some(path),
            state: Mutex::new(state),
            next_id: AtomicU64::new(next_id),
            pending: AtomicUsize::new(0),
            events,
        };
        record.rewrite(&mut *record.state.lock().await).await;
        record
    }

    #[cfg(test)]
    #[must_use]
    pub fn in_memory(events: EventSink) -> Self {
        Self {
            path: None,
            state: Mutex::new(State {
                entries: VecDeque::new(),
                lines: 0,
            }),
            next_id: AtomicU64::new(1),
            pending: AtomicUsize::new(0),
            events,
        }
    }

    /// Every entry kept, oldest first.
    pub async fn entries(&self) -> Vec<Entry> {
        let mut state = self.state.lock().await;
        // A running app may add nothing for days; age still counts.
        state.trim(Utc::now());
        state.entries.iter().cloned().collect()
    }

    /// Add an entry, unread. Returns its id.
    pub async fn add(&self, draft: Draft) -> u64 {
        let entry = self.number(draft);
        let id = entry.id;
        self.insert(entry).await;
        id
    }

    /// Number and date an entry now, to [`insert`](Self::insert) later: a
    /// caller that cannot wait for the write keeps the order of its events.
    #[must_use]
    pub fn number(&self, draft: Draft) -> Entry {
        let Draft {
            kind,
            namespace,
            revision,
            files,
            outcome,
        } = draft;
        Entry {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            kind,
            namespace,
            revision,
            files,
            outcome,
            at: Utc::now(),
            read: false,
        }
    }

    /// [`insert`](Self::insert) in the background, for a caller that cannot
    /// wait; [`settle`](Self::settle) waits for it.
    pub fn insert_later(self: &std::sync::Arc<Self>, entry: Entry) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        let record = std::sync::Arc::clone(self);
        crate::engine::events::spawn(async move {
            record.insert(entry).await;
            record.pending.fetch_sub(1, Ordering::SeqCst);
        });
    }

    /// Wait for the background writes, so a stop keeps what just happened,
    /// but not past a moment: a stop must not hang on a stuck disk.
    pub async fn settle(&self) {
        let written = async {
            while self.pending.load(Ordering::SeqCst) > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), written).await;
    }

    /// Keep an entry [numbered](Self::number) before.
    pub async fn insert(&self, entry: Entry) {
        info!(
            "record: added id={} kind={:?} namespace={}",
            entry.id, entry.kind, entry.namespace
        );
        let mut state = self.state.lock().await;
        state.insert(entry.clone());
        state.trim(Utc::now());
        self.append(&mut state, &Line::Add(entry)).await;
        drop(state);
        self.events.emit(RECORD_EVENT, ());
    }

    /// Mark read every entry up to `up_to`, only `namespace`'s when one is
    /// given. Entries added after the reader looked keep their flag.
    pub async fn mark_read(&self, up_to: u64, namespace: Option<Namespace>) {
        let mark = ReadMark { up_to, namespace };
        let mut state = self.state.lock().await;
        let mut changed = false;
        for entry in state
            .entries
            .iter_mut()
            .filter(|e| !e.read && mark.covers(e))
        {
            entry.read = true;
            changed = true;
        }
        if !changed {
            return;
        }
        self.append(&mut state, &Line::Read(mark)).await;
        drop(state);
        self.events.emit(RECORD_EVENT, ());
    }

    async fn append(&self, state: &mut State, line: &Line) {
        let Some(path) = &self.path else {
            return;
        };
        if state.lines >= 2 * MAX_ENTRIES.max(state.entries.len()) {
            self.rewrite(state).await;
            return;
        }
        let result = async {
            let mut text = serde_json::to_string(line).map_err(std::io::Error::other)?;
            text.push('\n');
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await?;
            file.write_all(text.as_bytes()).await?;
            file.flush().await
        }
        .await;
        match result {
            Ok(()) => state.lines += 1,
            Err(err) => warn!("record: could not append to {}: {err}", path.display()),
        }
    }

    /// Write the file anew from the entries kept: written beside it, then
    /// moved over it, so a crash leaves the old one whole.
    async fn rewrite(&self, state: &mut State) {
        let Some(path) = &self.path else {
            return;
        };
        let mut text = String::new();
        for entry in &state.entries {
            match serde_json::to_string(&Line::Add(entry.clone())) {
                Ok(line) => {
                    text.push_str(&line);
                    text.push('\n');
                }
                Err(err) => warn!("record: could not write entry {}: {err}", entry.id),
            }
        }
        let next = path.with_extension("jsonl.new");
        let result = async {
            tokio::fs::write(&next, text).await?;
            tokio::fs::rename(&next, path).await
        }
        .await;
        match result {
            Ok(()) => state.lines = state.entries.len(),
            Err(err) => warn!("record: could not rewrite {}: {err}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn ns() -> Namespace {
        ("acme", "demo").into()
    }

    fn other() -> Namespace {
        ("acme", "other").into()
    }

    fn report(added: &[&str]) -> PullReport {
        PullReport {
            manifest_uri: quilt_uri::ManifestUri {
                bucket: "b".to_string(),
                namespace: ns(),
                hash: "new".to_string(),
                origin: None,
            },
            added: added.iter().map(PathBuf::from).collect(),
            added_not_fetched: Vec::new(),
            updated: Vec::new(),
            removed: Vec::new(),
            kept_not_removed: Vec::new(),
            message: None,
        }
    }

    #[test]
    fn a_pull_keeps_its_groups_and_revisions() {
        let draft = Draft::got(
            &ns(),
            Some("old"),
            &PullReport {
                updated: vec![PathBuf::from("c.csv")],
                ..report(&["a.csv", "b.csv"])
            },
        );
        assert_eq!(draft.kind, Kind::Got);
        assert_eq!(
            draft.revision,
            Some(Revision {
                from: Some("old".into()),
                to: "new".into()
            })
        );
        assert_eq!(draft.files_line(), "2 files new, 1 file updated");
        assert_eq!(draft.files[0].paths, ["a.csv", "b.csv"]);
    }

    /// A 283-file pull keeps 200 paths and counts all 283.
    #[test]
    fn an_entry_keeps_its_counts_whole_and_some_paths() {
        let names: Vec<String> = (0..283).map(|n| format!("f{n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let draft = Draft::got(
            &ns(),
            None,
            &PullReport {
                removed: vec![PathBuf::from("gone.csv")],
                ..report(&names)
            },
        );
        assert_eq!(draft.files[0].count, 283);
        assert_eq!(draft.files[0].paths.len(), MAX_PATHS);
        assert_eq!(draft.files[1].count, 1);
        assert!(draft.files[1].paths.is_empty(), "no room left");
    }

    #[test]
    fn a_publish_groups_its_changes() {
        let row = quilt_rs::manifest::ManifestRow::default();
        let changes: ChangeSet = [
            (PathBuf::from("a"), Change::Added(row.clone())),
            (PathBuf::from("b"), Change::Modified(row.clone())),
            (PathBuf::from("c"), Change::Removed(row)),
        ]
        .into();
        let draft = Draft::published(&ns(), None, "h", &changes);
        assert_eq!(
            draft.files_line(),
            "1 file new, 1 file updated, 1 file removed"
        );
        assert_eq!(draft.revision.unwrap().from, None);
    }

    #[tokio::test]
    async fn entries_are_never_merged() {
        let record = Record::in_memory(EventSink::new());
        let a = record.add(Draft::got(&ns(), None, &report(&["a"]))).await;
        let b = record.add(Draft::got(&ns(), None, &report(&["a"]))).await;
        assert_ne!(a, b);
        assert_eq!(record.entries().await.len(), 2);
    }

    /// Two events numbered in order keep it, whichever is written first.
    #[tokio::test]
    async fn entries_keep_the_order_they_were_numbered_in() {
        let record = Record::in_memory(EventSink::new());
        let published = record.number(Draft::uninstalled(&ns(), None));
        let got = record.number(Draft::uninstalled(&other(), None));
        record.insert(got).await;
        record.insert(published).await;
        let ids: Vec<u64> = record.entries().await.iter().map(|e| e.id).collect();
        assert_eq!(ids, [1, 2]);
    }

    #[tokio::test]
    async fn reading_stops_at_what_the_reader_saw() {
        let record = Record::in_memory(EventSink::new());
        let first = record.add(Draft::uninstalled(&ns(), None)).await;
        let theirs = record.add(Draft::uninstalled(&other(), None)).await;
        record.add(Draft::uninstalled(&ns(), None)).await;

        record.mark_read(theirs, Some(ns())).await;
        let read: Vec<bool> = record.entries().await.iter().map(|e| e.read).collect();
        assert_eq!(
            read,
            [true, false, false],
            "only acme/demo, only up to {first}"
        );

        record.mark_read(u64::MAX, None).await;
        assert!(record.entries().await.iter().all(|e| e.read));
    }

    #[tokio::test]
    async fn a_change_is_announced() {
        let events = EventSink::new();
        let mut rx = events.subscribe();
        let record = Record::in_memory(events);
        let id = record.add(Draft::uninstalled(&ns(), None)).await;
        assert_eq!(rx.recv().await.unwrap().0, RECORD_EVENT);
        record.mark_read(id, None).await;
        assert_eq!(rx.recv().await.unwrap().0, RECORD_EVENT);
        // Nothing left to read: nothing changed, nothing said.
        record.mark_read(id, None).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_record_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let record = Record::open(dir.path(), EventSink::new()).await;
        let a = record
            .add(Draft::uninstalled(&ns(), Some("Freed 1 kB.".into())))
            .await;
        record.add(Draft::uninstalled(&other(), None)).await;
        record.mark_read(a, None).await;
        drop(record);

        let again = Record::open(dir.path(), EventSink::new()).await;
        let entries = again.entries().await;
        assert_eq!(entries.len(), 2);
        assert!(entries[0].read);
        assert!(!entries[1].read);
        assert_eq!(entries[0].outcome.as_deref(), Some("Freed 1 kB."));
        // Ids go on from where they were.
        assert!(again.add(Draft::uninstalled(&ns(), None)).await > entries[1].id);
    }

    /// A record emptied by age still hands out ids past the ones before.
    #[tokio::test]
    async fn an_emptied_record_does_not_reuse_ids() {
        let dir = tempfile::tempdir().unwrap();
        let record = Record::open(dir.path(), EventSink::new()).await;
        let mut old = record.number(Draft::uninstalled(&ns(), None));
        old.at = Utc::now() - KEEP_FOR - chrono::TimeDelta::hours(1);
        let old_id = old.id;
        record.insert(old).await;
        drop(record);
        // A restart takes longer than the millisecond the ids count in.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let again = Record::open(dir.path(), EventSink::new()).await;
        assert_eq!(again.entries().await, []);
        assert!(again.add(Draft::uninstalled(&ns(), None)).await > old_id);
    }

    /// A file that cannot be read is left whole, and nothing is written over
    /// it this run.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_file_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "kept\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let record = Record::open(dir.path(), EventSink::new()).await;
        record.add(Draft::uninstalled(&ns(), None)).await;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept\n");
        assert_eq!(record.entries().await.len(), 1, "kept in memory");
    }

    /// An append cut short inside a character costs that line, not the rest.
    #[tokio::test]
    async fn a_line_cut_inside_a_character_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let record = Record::open(dir.path(), EventSink::new()).await;
        record.add(Draft::uninstalled(&ns(), None)).await;
        drop(record);
        let path = dir.path().join(FILE_NAME);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{\"add\":{\"outcome\":\"caf\xc3");
        std::fs::write(&path, bytes).unwrap();

        let again = Record::open(dir.path(), EventSink::new()).await;
        assert_eq!(again.entries().await.len(), 1);
    }

    #[tokio::test]
    async fn a_line_that_does_not_parse_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let record = Record::open(dir.path(), EventSink::new()).await;
        record.add(Draft::uninstalled(&ns(), None)).await;
        drop(record);
        let path = dir.path().join(FILE_NAME);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{not json\n");
        std::fs::write(&path, text).unwrap();

        let again = Record::open(dir.path(), EventSink::new()).await;
        assert_eq!(again.entries().await.len(), 1);
    }

    fn entry_at(id: u64, at: DateTime<Utc>) -> Entry {
        Entry {
            id,
            kind: Kind::Uninstalled,
            namespace: ns(),
            revision: None,
            files: Vec::new(),
            outcome: None,
            at,
            read: false,
        }
    }

    /// A running app that adds nothing still lets old entries go.
    #[tokio::test]
    async fn reading_drops_what_has_aged_out() {
        let record = Record::in_memory(EventSink::new());
        let mut old = record.number(Draft::uninstalled(&ns(), None));
        old.at = Utc::now() - KEEP_FOR - chrono::TimeDelta::hours(1);
        record.insert(old).await;
        assert_eq!(record.entries().await, []);
    }

    #[test]
    fn the_oldest_go_first_past_either_bound() {
        let now = Utc::now();
        let mut state = State {
            entries: VecDeque::new(),
            lines: 0,
        };
        state.apply(Line::Add(entry_at(
            1,
            now - KEEP_FOR - chrono::Duration::hours(1),
        )));
        for id in 2..=(MAX_ENTRIES as u64 + 2) {
            state.apply(Line::Add(entry_at(id, now)));
        }
        state.trim(now);
        assert_eq!(state.entries.len(), MAX_ENTRIES);
        assert_eq!(state.entries.front().unwrap().id, 3);
    }

    /// The window matches on these names; a rename here breaks it silently.
    #[test]
    fn an_entry_reads_as_the_window_expects() {
        let mut entry = entry_at(7, DateTime::UNIX_EPOCH);
        entry.kind = Kind::RemovedRevisions { count: 2 };
        entry.files = vec![FileGroup {
            change: FileChange::NewNotDownloaded,
            count: 1,
            paths: vec!["a".into()],
        }];
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({
                "id": 7,
                "kind": "removedRevisions",
                "count": 2,
                "namespace": "acme/demo",
                "revision": null,
                "files": [{"change": "newNotDownloaded", "count": 1, "paths": ["a"]}],
                "outcome": null,
                "at": "1970-01-01T00:00:00Z",
                "read": false,
            })
        );
    }
}
