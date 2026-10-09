//! The durable state between "boundary decided" and "revision published"
//! is an append-only journal, one fsync'd JSON line per state
//! change. A run's state is the fold of its events and is never stored
//! separately, so a `kill -9` at any point resumes from the last line written.

use std::collections::BTreeMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::sentinel::FileEntry;

/// One journal line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Membership frozen at the boundary; everything a resume needs to finish.
    Snapshot {
        run_id: String,
        instrument_id: String,
        folder: PathBuf,
        sentinel_id: String,
        boundary: serde_json::Value,
        instrument_local_time: Option<String>,
        snapshot_time_utc: String,
        total_bytes: u64,
        /// Relative paths, sizes and mtimes as seen at the boundary.
        members: Vec<SnapMember>,
    },
    /// The run is over the size cap and will not be uploaded.
    Refused { run_id: String, reason: String },
    /// One member uploaded and checked against the snapshot.
    MemberVerified { run_id: String, file: FileEntry },
    /// The sentinel is in S3.
    Sealed {
        run_id: String,
        sentinel_key: String,
        previous_sentinel_id: Option<String>,
    },
    /// The revision is published.
    Landed {
        run_id: String,
        package_name: String,
        top_hash: String,
        latest_advanced: bool,
    },
    /// The source changed after the boundary; the run is parked for an operator.
    Suspect { run_id: String, reason: String },
}

impl Event {
    #[must_use]
    pub fn run_id(&self) -> &str {
        match self {
            Event::Snapshot { run_id, .. }
            | Event::Refused { run_id, .. }
            | Event::MemberVerified { run_id, .. }
            | Event::Sealed { run_id, .. }
            | Event::Landed { run_id, .. }
            | Event::Suspect { run_id, .. } => run_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapMember {
    pub path: String,
    pub size: u64,
    pub mtime_unix: i64,
}

/// Where a run stands, folded from its events.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Uploading,
    Sealed,
    Landed,
    Refused,
    Suspect,
}

/// The newest capture of a folder: what it froze, and when.
#[derive(Debug, Clone)]
pub struct Captured {
    pub members: Vec<SnapMember>,
    pub at: Option<SystemTime>,
}

/// A run's folded events.
#[derive(Debug, Clone)]
pub struct Run {
    pub snapshot: Event,
    pub verified: BTreeMap<String, FileEntry>,
    pub sealed: Option<(String, Option<String>)>,
    /// Journal line of the `Sealed` event, for ordering the chain by seal time.
    pub sealed_at_line: usize,
    /// Journal line of the `Snapshot`: the order runs were captured in.
    pub captured_at_line: usize,
    /// `(package, top_hash)` once landed.
    pub landed: Option<(String, String)>,
    /// Why the run is parked, when it is `Refused` or `Suspect`.
    pub reason: Option<String>,
    pub state: RunState,
}

#[derive(Debug)]
pub struct Spool {
    root: PathBuf,
    journal: File,
}

impl Spool {
    pub fn open(root: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(root)?;
        let path = root.join("journal.jsonl");
        // A kill mid-write leaves a line with no newline. Cut it off before
        // appending, or the next event is glued to it and lost on every read.
        if let Ok(bytes) = std::fs::read(&path)
            && let Some(last) = bytes.last()
            && *last != b'\n'
        {
            let keep = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
            OpenOptions::new()
                .write(true)
                .open(&path)?
                .set_len(keep as u64)?;
        }
        let journal = OpenOptions::new().create(true).append(true).open(&path)?;
        // One agent per spool: a second process would snapshot the same runs.
        // The OS releases the lock when this process exits, however it exits.
        journal.try_lock().map_err(|_| {
            Error::Refused(format!(
                "another quilt-agent is using {}; stop it first",
                root.display()
            ))
        })?;
        Ok(Spool {
            root: root.to_path_buf(),
            journal,
        })
    }

    /// The runs in the journal under `root`, read without taking the lock, so
    /// `quilt-agent status` works while the service is running.
    pub fn read_only(root: &Path) -> Result<BTreeMap<String, Run>, Error> {
        Spool {
            root: root.to_path_buf(),
            journal: File::open(root.join("journal.jsonl"))?,
        }
        .runs()
    }

    /// Append and fsync one event; it is durable when this returns.
    pub fn record(&mut self, event: &Event) -> Result<(), Error> {
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        self.journal.write_all(&line)?;
        self.journal.sync_data()?;
        Ok(())
    }

    /// Every run in the journal, by run id. A torn final line (a kill mid-write)
    /// is dropped: the event it carried was never acknowledged.
    pub fn runs(&self) -> Result<BTreeMap<String, Run>, Error> {
        let file = File::open(self.root.join("journal.jsonl"))?;
        let mut runs: BTreeMap<String, Run> = BTreeMap::new();
        for (line_no, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            let Ok(event) = serde_json::from_str::<Event>(&line) else {
                tracing::warn!("skipping unreadable journal line");
                continue;
            };
            let id = event.run_id().to_string();
            match event {
                Event::Snapshot { .. } => {
                    runs.insert(
                        id,
                        Run {
                            snapshot: event,
                            verified: BTreeMap::new(),
                            sealed: None,
                            sealed_at_line: 0,
                            captured_at_line: line_no,
                            landed: None,
                            reason: None,
                            state: RunState::Uploading,
                        },
                    );
                }
                Event::MemberVerified { file, .. } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.verified.insert(file.path.clone(), file);
                    }
                }
                Event::Sealed {
                    sentinel_key,
                    previous_sentinel_id,
                    ..
                } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.sealed = Some((sentinel_key, previous_sentinel_id));
                        run.sealed_at_line = line_no;
                        run.state = RunState::Sealed;
                    }
                }
                Event::Landed {
                    package_name,
                    top_hash,
                    ..
                } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.landed = Some((package_name, top_hash));
                        run.state = RunState::Landed;
                    }
                }
                Event::Suspect { reason, .. } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.reason = Some(reason);
                        run.state = RunState::Suspect;
                    }
                }
                Event::Refused { reason, .. } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.reason = Some(reason);
                        run.state = RunState::Refused;
                    }
                }
            }
        }
        Ok(runs)
    }

    /// The last sentinel sealed for `instrument_id`, for the chain.
    /// Ordered by when it was sealed, not when its run began: a run that
    /// retried for hours and sealed late is still the newest link.
    pub fn previous_sentinel(&self, instrument_id: &str) -> Result<Option<String>, Error> {
        let runs = self.runs()?;
        Ok(runs
            .values()
            .filter(|r| r.sealed.is_some())
            .filter_map(|r| match &r.snapshot {
                Event::Snapshot {
                    instrument_id: i,
                    sentinel_id,
                    ..
                } if i == instrument_id => Some((r.sealed_at_line, sentinel_id.clone())),
                _ => None,
            })
            .max()
            .map(|(_, id)| id))
    }

    /// The parent for `run_id`'s revision of `package` (`bucket:name`): the
    /// newest of this agent's runs of that package captured *before* it. An
    /// older run retried after a newer one landed must not name the newer
    /// one as its parent, or its publish would move `latest` backward.
    pub fn parent_for(&self, package: &str, run_id: &str) -> Result<Option<String>, Error> {
        let runs = self.runs()?;
        let Some(mine) = runs.get(run_id).map(|r| r.captured_at_line) else {
            return Ok(None);
        };
        Ok(runs
            .values()
            .filter(|r| r.captured_at_line < mine)
            .filter_map(|r| match &r.landed {
                Some((p, hash)) if p == package => Some((r.captured_at_line, hash.clone())),
                _ => None,
            })
            .max()
            .map(|(_, hash)| hash))
    }

    /// The newest snapshot of each folder: the members it froze. A scan uses
    /// it to tell a captured run (still there, maybe grown) from a new run
    /// the instrument wrote at the same path.
    pub fn known_folders(&self) -> Result<BTreeMap<PathBuf, Captured>, Error> {
        let mut runs: Vec<Run> = self.runs()?.into_values().collect();
        runs.sort_by_key(|r| r.captured_at_line);
        Ok(runs
            .into_iter()
            .filter_map(|r| match r.snapshot {
                Event::Snapshot {
                    folder,
                    members,
                    snapshot_time_utc,
                    ..
                } => Some((
                    folder,
                    Captured {
                        members,
                        at: chrono::DateTime::parse_from_rfc3339(&snapshot_time_utc)
                            .ok()
                            .map(SystemTime::from),
                    },
                )),
                _ => None,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn snapshot(run_id: &str, instrument: &str, at: &str) -> Event {
        Event::Snapshot {
            run_id: run_id.to_string(),
            instrument_id: instrument.to_string(),
            folder: PathBuf::from(format!("/data/{run_id}")),
            sentinel_id: format!("sentinel-{run_id}"),
            boundary: serde_json::json!({}),
            instrument_local_time: None,
            snapshot_time_utc: at.to_string(),
            total_bytes: 3,
            members: vec![],
        }
    }

    fn file(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            key: format!("lab/i/r/{path}"),
            size: 3,
            checksum: crate::sentinel::Checksum {
                algorithm: "sha2-256-chunked".to_string(),
                value: "abc".to_string(),
            },
            mtime_local: None,
            version_id: Some("v1".to_string()),
        }
    }

    /// `status` reads a running agent's journal and shows why a run is parked.
    #[test]
    fn status_reads_past_the_lock_and_keeps_the_reason() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let mut spool = Spool::open(dir.path())?;
        spool.record(&snapshot("r1", "i", "2026-10-07T00:00:00Z"))?;
        spool.record(&Event::Suspect {
            run_id: "r1".into(),
            reason: "member changed after boundary: a.fcs".into(),
        })?;
        let runs = Spool::read_only(dir.path())?;
        assert_eq!(runs["r1"].state, RunState::Suspect);
        assert_eq!(
            runs["r1"].reason.as_deref(),
            Some("member changed after boundary: a.fcs")
        );
        Ok(())
    }

    #[test]
    fn a_second_agent_cannot_open_the_same_spool() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let _first = Spool::open(dir.path())?;
        assert!(Spool::open(dir.path()).is_err());
        Ok(())
    }

    #[test]
    fn state_is_the_fold_of_events() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let mut spool = Spool::open(dir.path())?;
        spool.record(&snapshot("r1", "i", "2026-10-07T00:00:00Z"))?;
        spool.record(&Event::MemberVerified {
            run_id: "r1".into(),
            file: file("a"),
        })?;
        let r = &spool.runs()?["r1"];
        assert_eq!(r.state, RunState::Uploading);
        assert_eq!(r.verified.len(), 1);

        spool.record(&Event::Sealed {
            run_id: "r1".into(),
            sentinel_key: "k".into(),
            previous_sentinel_id: None,
        })?;
        assert_eq!(spool.runs()?["r1"].state, RunState::Sealed);
        Ok(())
    }

    /// A process killed mid-write leaves a partial last line; the journal still
    /// reads, and the half-written event never happened.
    #[test]
    fn a_torn_last_line_is_dropped() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let mut spool = Spool::open(dir.path())?;
        spool.record(&snapshot("r1", "i", "2026-10-07T00:00:00Z"))?;
        drop(spool);
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.path().join("journal.jsonl"))?;
        f.write_all(br#"{"event":"sealed","run_id":"r1","sentinel_ke"#)?;
        let mut spool = Spool::open(dir.path())?;
        assert_eq!(spool.runs()?["r1"].state, RunState::Uploading);
        // The first event after reopening is not glued to the torn line.
        spool.record(&Event::Landed {
            run_id: "r1".into(),
            package_name: "a/b".into(),
            top_hash: "h".into(),
            latest_advanced: true,
        })?;
        assert_eq!(spool.runs()?["r1"].state, RunState::Landed);
        Ok(())
    }

    fn landed(run: &str, hash: &str) -> Event {
        Event::Landed {
            run_id: run.into(),
            package_name: "raw:a/b".into(),
            top_hash: hash.into(),
            latest_advanced: true,
        }
    }

    /// Run A is captured before B; B lands first; A's retry must not take B
    /// as its parent, or it would move `latest` back to the older run.
    #[test]
    fn an_older_run_retried_late_never_parents_on_a_newer_one() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let mut spool = Spool::open(dir.path())?;
        spool.record(&snapshot("a", "i", "2026-10-07T00:00:00Z"))?;
        spool.record(&snapshot("b", "i", "2026-10-07T01:00:00Z"))?;
        spool.record(&landed("b", "hash-b"))?;
        assert_eq!(spool.parent_for("raw:a/b", "a")?, None);
        spool.record(&landed("a", "hash-a"))?;
        assert_eq!(spool.parent_for("raw:a/b", "b")?.as_deref(), Some("hash-a"));
        spool.record(&snapshot("c", "i", "2026-10-07T02:00:00Z"))?;
        assert_eq!(spool.parent_for("raw:a/b", "c")?.as_deref(), Some("hash-b"));
        Ok(())
    }

    /// A run that seals after a newer run is the newest link; the chain
    /// must not fork back to the run that started later.
    #[test]
    fn chain_follows_seal_order_not_start_order() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let mut spool = Spool::open(dir.path())?;
        spool.record(&snapshot("old", "i", "2026-10-07T00:00:00Z"))?;
        spool.record(&snapshot("new", "i", "2026-10-07T01:00:00Z"))?;
        for run in ["new", "old"] {
            spool.record(&Event::Sealed {
                run_id: run.into(),
                sentinel_key: "k".into(),
                previous_sentinel_id: None,
            })?;
        }
        assert_eq!(
            spool.previous_sentinel("i")?.as_deref(),
            Some("sentinel-old")
        );
        Ok(())
    }

    #[test]
    fn chain_names_the_last_sealed_run_per_instrument() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let mut spool = Spool::open(dir.path())?;
        for (run, at) in [
            ("r1", "2026-10-07T00:00:00Z"),
            ("r2", "2026-10-07T01:00:00Z"),
        ] {
            spool.record(&snapshot(run, "i", at))?;
            spool.record(&Event::Sealed {
                run_id: run.into(),
                sentinel_key: "k".into(),
                previous_sentinel_id: None,
            })?;
        }
        spool.record(&snapshot("r3", "i", "2026-10-07T02:00:00Z"))?;
        assert_eq!(
            spool.previous_sentinel("i")?.as_deref(),
            Some("sentinel-r2")
        );
        assert_eq!(spool.previous_sentinel("other")?, None);
        Ok(())
    }
}
