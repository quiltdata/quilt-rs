//! The hashes of a package's files from earlier walks, keyed by each file's
//! [`FileStamp`], so a walk hashes only the files that look changed.
//!
//! Optional: a walk without one hashes every file, and gets the same answer.
//! An entry is trusted only when every stamp value matches and the platform
//! gave a change time and a file ID. Tools that restore an edit time (`touch -r`,
//! `cp -p`, `rsync -a`) cannot restore the change time, so they are seen.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use crate::file_stamp::FileStamp;
use crate::object_hash::ObjectHash;

/// How long a package goes between walks that hash every file anyway.
pub const FULL_HASH_EVERY: Duration = Duration::from_hours(24);

/// A file's hash with one algorithm, and the stamp it was taken at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedHash {
    pub stamp: FileStamp,
    pub hash: ObjectHash,
    /// When the stamp was read.
    pub saved_at: SystemTime,
}

impl CachedHash {
    /// Racy, as git calls it: edited in the same second the entry was saved,
    /// so an edit right after the stamp was read could share its edit time.
    fn racy(&self) -> bool {
        whole_secs(self.stamp.modified) >= whole_secs(self.saved_at)
    }
}

/// One package's cache for one walk: what earlier walks saved, and what this
/// walk changes.
#[derive(Debug, Default)]
pub struct StatCache {
    saved: HashMap<(PathBuf, u64), CachedHash>,
    /// Hash every file this walk, the daily safety net.
    full: bool,
    seen: HashSet<PathBuf>,
    updates: Vec<(PathBuf, CachedHash)>,
}

impl StatCache {
    /// `saved` is what the store holds, by logical key and multihash algorithm.
    #[must_use]
    pub fn new(saved: HashMap<(PathBuf, u64), CachedHash>, full: bool) -> Self {
        Self {
            saved,
            full,
            ..Self::default()
        }
    }

    /// Whether this walk hashes every file.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// The saved hash of `key` with `algorithm`, if `stamp` says the file is
    /// the one it was taken from.
    pub(crate) fn lookup(
        &self,
        key: &Path,
        stamp: &FileStamp,
        algorithm: u64,
    ) -> Option<&ObjectHash> {
        if self.full || !trusted(stamp) {
            return None;
        }
        self.saved
            .get(&(key.to_path_buf(), algorithm))
            .filter(|entry| entry.stamp == *stamp && !entry.racy())
            .map(|entry| &entry.hash)
    }

    /// The walk found `key`.
    pub(crate) fn see(&mut self, key: &Path) {
        self.seen.insert(key.to_path_buf());
    }

    /// The walk hashed `key`. Kept only when it differs from what is saved, or
    /// the saved entry was racy and this one isn't.
    pub(crate) fn record(&mut self, key: &Path, entry: CachedHash) {
        if !trusted(&entry.stamp) || key.to_str().is_none() {
            return;
        }
        let unchanged = self
            .saved
            .get(&(key.to_path_buf(), entry.hash.algorithm()))
            .is_some_and(|saved| {
                saved.stamp == entry.stamp && saved.hash == entry.hash && !saved.racy()
            });
        if !unchanged {
            self.updates.push((key.to_path_buf(), entry));
        }
    }

    /// The entries to write.
    #[must_use]
    pub fn updates(&self) -> &[(PathBuf, CachedHash)] {
        &self.updates
    }

    /// The saved paths this walk did not find, to delete.
    #[must_use]
    pub fn gone(&self) -> Vec<PathBuf> {
        let mut gone: Vec<PathBuf> = self
            .saved
            .keys()
            .map(|(path, _)| path)
            .filter(|path| !self.seen.contains(*path))
            .cloned()
            .collect();
        gone.sort();
        gone.dedup();
        gone
    }
}

/// Without a change time and a file ID, a matching size and edit time prove
/// nothing.
fn trusted(stamp: &FileStamp) -> bool {
    stamp.changed.is_some() && stamp.id.is_some()
}

fn whole_secs(at: SystemTime) -> i64 {
    match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            let before = before.duration();
            let secs = i64::try_from(before.as_secs()).unwrap_or(i64::MAX);
            if before.subsec_nanos() == 0 {
                -secs
            } else {
                -secs - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::file_stamp::FileId;
    use crate::object_hash::Crc64Hash;
    use crate::object_hash::MULTIHASH_CRC64_NVME;

    fn at(secs: u64, nanos: u32) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::new(secs, nanos)
    }

    fn stamp(modified: SystemTime) -> FileStamp {
        FileStamp {
            size: 3,
            modified,
            changed: Some(modified),
            id: Some(FileId { volume: 1, file: 7 }),
        }
    }

    fn hash() -> ObjectHash {
        Crc64Hash::default().into()
    }

    fn cache_with(entry: CachedHash, full: bool) -> StatCache {
        StatCache::new(
            HashMap::from([((PathBuf::from("a"), MULTIHASH_CRC64_NVME), entry)]),
            full,
        )
    }

    #[test]
    fn a_matching_stamp_hits() {
        let entry = CachedHash {
            stamp: stamp(at(100, 0)),
            hash: hash(),
            saved_at: at(200, 0),
        };
        let cache = cache_with(entry, false);
        assert_eq!(
            cache.lookup(Path::new("a"), &stamp(at(100, 0)), MULTIHASH_CRC64_NVME),
            Some(&hash())
        );
    }

    #[test]
    fn any_stamp_value_that_differs_misses() {
        let saved = stamp(at(100, 0));
        let cache = cache_with(
            CachedHash {
                stamp: saved,
                hash: hash(),
                saved_at: at(200, 0),
            },
            false,
        );
        let changes = [
            FileStamp { size: 4, ..saved },
            FileStamp {
                modified: at(100, 1),
                ..saved
            },
            // `touch -r` or `cp -p`: the edit time restored, the change time new.
            FileStamp {
                changed: Some(at(150, 0)),
                ..saved
            },
            FileStamp {
                id: Some(FileId { volume: 1, file: 8 }),
                ..saved
            },
        ];
        for changed in changes {
            assert_eq!(
                cache.lookup(Path::new("a"), &changed, MULTIHASH_CRC64_NVME),
                None,
                "{changed:?}"
            );
        }
    }

    #[test]
    fn another_algorithm_misses() {
        let cache = cache_with(
            CachedHash {
                stamp: stamp(at(100, 0)),
                hash: hash(),
                saved_at: at(200, 0),
            },
            false,
        );
        assert_eq!(
            cache.lookup(
                Path::new("a"),
                &stamp(at(100, 0)),
                crate::object_hash::MULTIHASH_SHA256_CHUNKED
            ),
            None
        );
    }

    #[test]
    fn an_edit_in_the_second_it_was_saved_misses() {
        let cache = cache_with(
            CachedHash {
                stamp: stamp(at(100, 900_000_000)),
                hash: hash(),
                saved_at: at(100, 950_000_000),
            },
            false,
        );
        assert_eq!(
            cache.lookup(
                Path::new("a"),
                &stamp(at(100, 900_000_000)),
                MULTIHASH_CRC64_NVME
            ),
            None
        );
    }

    #[test]
    fn a_full_walk_misses() {
        let cache = cache_with(
            CachedHash {
                stamp: stamp(at(100, 0)),
                hash: hash(),
                saved_at: at(200, 0),
            },
            true,
        );
        assert_eq!(
            cache.lookup(Path::new("a"), &stamp(at(100, 0)), MULTIHASH_CRC64_NVME),
            None
        );
    }

    #[test]
    fn a_stamp_without_change_time_or_id_is_never_trusted() {
        let untrusted = FileStamp {
            changed: None,
            ..stamp(at(100, 0))
        };
        let mut cache = cache_with(
            CachedHash {
                stamp: untrusted,
                hash: hash(),
                saved_at: at(200, 0),
            },
            false,
        );
        assert_eq!(
            cache.lookup(Path::new("a"), &untrusted, MULTIHASH_CRC64_NVME),
            None
        );
        cache.record(
            Path::new("b"),
            CachedHash {
                stamp: untrusted,
                hash: hash(),
                saved_at: at(200, 0),
            },
        );
        assert_eq!(cache.updates(), &[]);
    }

    #[test]
    fn only_changed_values_are_written() {
        let entry = CachedHash {
            stamp: stamp(at(100, 0)),
            hash: hash(),
            saved_at: at(200, 0),
        };
        let mut cache = cache_with(entry.clone(), false);
        cache.record(Path::new("a"), entry.clone());
        assert_eq!(cache.updates(), &[], "the same values");

        let newer = CachedHash {
            stamp: stamp(at(300, 0)),
            saved_at: at(400, 0),
            ..entry
        };
        cache.record(Path::new("a"), newer.clone());
        assert_eq!(cache.updates(), &[(PathBuf::from("a"), newer)]);
    }

    #[test]
    fn a_racy_entry_is_written_again_once_it_is_not() {
        let racy = CachedHash {
            stamp: stamp(at(100, 0)),
            hash: hash(),
            saved_at: at(100, 500_000_000),
        };
        let mut cache = cache_with(racy.clone(), false);
        let settled = CachedHash {
            saved_at: at(101, 0),
            ..racy
        };
        cache.record(Path::new("a"), settled.clone());
        assert_eq!(cache.updates(), &[(PathBuf::from("a"), settled)]);
    }

    #[test]
    fn saved_paths_the_walk_did_not_find_are_gone() {
        let mut cache = cache_with(
            CachedHash {
                stamp: stamp(at(100, 0)),
                hash: hash(),
                saved_at: at(200, 0),
            },
            false,
        );
        assert_eq!(cache.gone(), vec![PathBuf::from("a")]);
        cache.see(Path::new("a"));
        assert_eq!(cache.gone(), Vec::<PathBuf>::new());
    }
}
