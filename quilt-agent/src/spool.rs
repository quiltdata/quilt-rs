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
    /// The run is too big for the spool and was not staged.
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
#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
    Uploading,
    Sealed,
    Landed,
    Refused,
    Suspect,
}

/// A run's folded events.
#[derive(Debug, Clone)]
pub struct Run {
    pub snapshot: Event,
    pub verified: BTreeMap<String, FileEntry>,
    pub sealed: Option<(String, Option<String>)>,
    /// Journal line of the `Sealed` event, for ordering the chain by seal time.
    pub sealed_at_line: usize,
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
        Ok(Spool {
            root: root.to_path_buf(),
            journal,
        })
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
                Event::Landed { .. } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.state = RunState::Landed;
                    }
                }
                Event::Suspect { .. } => {
                    if let Some(run) = runs.get_mut(&id) {
                        run.state = RunState::Suspect;
                    }
                }
                Event::Refused { .. } => {
                    runs.entry(id).and_modify(|r| r.state = RunState::Refused);
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

    /// The top hash this agent last landed for `package` (`bucket:name`).
    pub fn last_landed(&self, package: &str) -> Result<Option<String>, Error> {
        let file = File::open(self.root.join("journal.jsonl"))?;
        let mut last = None;
        for line in BufReader::new(file).lines() {
            if let Ok(Event::Landed {
                package_name,
                top_hash,
                ..
            }) = serde_json::from_str(&line?)
                && package_name == package
            {
                last = Some(top_hash);
            }
        }
        Ok(last)
    }

    /// Folders already observed, with how many members their snapshot froze,
    /// so a scan neither snapshots a run twice nor misses files added after.
    pub fn known_folders(&self) -> Result<BTreeMap<PathBuf, usize>, Error> {
        Ok(self
            .runs()?
            .values()
            .filter_map(|r| match &r.snapshot {
                Event::Snapshot {
                    folder, members, ..
                } => Some((folder.clone(), members.len())),
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
