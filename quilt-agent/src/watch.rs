//! Scans an instrument's source for run folders and their members.
//!
//! Polling is the source of truth: over SMB, `notify` delivers nothing usable
//! and a directory listing can be a minute stale (SP-1). The run loop uses
//! `notify` only to wake early; a missed event costs at most one poll.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use globset::Glob;
use globset::GlobSet;
use globset::GlobSetBuilder;

use crate::Error;
use crate::profile::Source;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Relative to the run folder, with the platform separator.
    pub path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
}

pub fn glob_set(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p)?);
    }
    builder.build()
}

/// The run folders directly under `source.path` that match `run_folder_glob`.
pub fn run_folders(source: &Source) -> Result<Vec<PathBuf>, Error> {
    let select = glob_set(std::slice::from_ref(&source.run_folder_glob))?;
    let mut runs = Vec::new();
    for entry in std::fs::read_dir(&source.path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && select.is_match(entry.file_name()) {
            runs.push(entry.path());
        }
    }
    runs.sort();
    Ok(runs)
}

/// Every file under `folder` except the ignored ones, sorted by path.
pub fn members(folder: &Path, ignore: &GlobSet) -> Result<Vec<Member>, Error> {
    let mut out = BTreeMap::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let Ok(rel) = path.strip_prefix(folder).map(Path::to_path_buf) else {
                continue;
            };
            if ignore.is_match(&rel) || rel.file_name().is_some_and(|n| ignore.is_match(n)) {
                continue;
            }
            let meta = entry.metadata()?;
            out.insert(
                rel.clone(),
                Member {
                    path: rel,
                    size: meta.len(),
                    mtime: meta.modified()?,
                },
            );
        }
    }
    Ok(out.into_values().collect())
}

/// A run folder as seen across scans: when its listing last changed.
#[derive(Debug, Clone)]
pub struct Tracked {
    pub members: Vec<Member>,
    pub last_change: SystemTime,
}

impl Tracked {
    /// Fold a fresh scan in: any added, removed, resized or touched member
    /// moves `last_change` to `now`.
    #[must_use]
    pub fn update(previous: Option<Tracked>, members: Vec<Member>, now: SystemTime) -> Tracked {
        match previous {
            Some(prev) if prev.members == members => prev,
            _ => Tracked {
                members,
                last_change: now,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    #[test]
    fn scans_runs_and_members_skipping_ignored() -> Result<(), Error> {
        let dir = tempfile::tempdir()?;
        let run = dir.path().join("2026-10-07_plate1");
        std::fs::create_dir_all(run.join("sub"))?;
        std::fs::write(run.join("a.fcs"), b"aaa")?;
        std::fs::write(run.join("sub/b.fcs"), b"bb")?;
        std::fs::write(run.join("Thumbs.db"), b"x")?;
        std::fs::write(dir.path().join("stray.txt"), b"outside any run")?;

        let source = Source {
            path: dir.path().to_path_buf(),
            run_folder_glob: "*".to_string(),
            ignore: vec!["Thumbs.db".to_string()],
            poll_s: 30,
            dir_cache_ttl_s: 0,
        };
        assert_eq!(run_folders(&source)?, vec![run.clone()]);
        let found = members(&run, &glob_set(&source.ignore)?)?;
        let paths: Vec<_> = found.iter().map(|m| m.path.clone()).collect();
        assert_eq!(
            paths,
            vec![PathBuf::from("a.fcs"), PathBuf::from("sub/b.fcs")]
        );
        assert_eq!(found[0].size, 3);
        Ok(())
    }

    #[test]
    fn last_change_moves_only_when_the_listing_does() {
        let t = |s| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let m = vec![Member {
            path: "a".into(),
            size: 1,
            mtime: t(1),
        }];
        let first = Tracked::update(None, m.clone(), t(10));
        let same = Tracked::update(Some(first), m.clone(), t(20));
        assert_eq!(same.last_change, t(10));
        let grown = vec![Member {
            size: 2,
            ..m[0].clone()
        }];
        assert_eq!(Tracked::update(Some(same), grown, t(30)).last_change, t(30));
    }
}
