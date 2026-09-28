//! How a writer of one package's entry applies its own change.
//!
//! Each writer reads the entry once, at its start, and does its slow work from
//! that copy. At its write, under the lock, it compares that copy with the
//! entry it finds now. If nothing it depends on moved it applies only its own
//! change ([`own_change`]); otherwise it redoes the missing part, at most
//! [`MAX_REDO`] times, or refuses. Which of those is the writer's own rule; the
//! pieces they share live here.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use multihash::Multihash;

use crate::Error;
use crate::InstallPackageError;
use crate::checksum::refresh_hash;
use crate::error::PackageOpError;
use crate::io::storage::Storage;
use crate::lineage::LineagePaths;
use crate::lineage::PackageLineage;
use crate::manifest::ManifestRow;
use crate::object_hash::ObjectHash;
use quilt_uri::Namespace;

/// How many times a writer redoes its work against an entry that moved before
/// it refuses. One: a second crossing in a row means something is writing in a
/// loop, and the user can press again.
pub(super) const MAX_REDO: usize = 1;

/// What a writer's step found under the lock.
pub(super) enum Verdict<T> {
    /// It wrote (or had nothing to write) and is done.
    Written(T),
    /// The entry moved in a way its work missed: redo against this copy.
    Redo(PackageLineage),
    /// The entry moved in a way its work cannot be made true of.
    Refused,
    /// The package was uninstalled while it worked.
    Gone,
}

pub(super) fn changed_underneath(namespace: &Namespace, verb: &'static str) -> Error {
    PackageOpError::ChangedUnderneath {
        namespace: namespace.clone(),
        verb,
    }
    .into()
}

pub(super) fn not_installed(namespace: &Namespace) -> Error {
    Error::InstallPackage(InstallPackageError::NotInstalled(namespace.clone()))
}

/// The entry's **revision**: what `current_hash()` and the upstream state are
/// computed from.
pub(super) fn same_revision(a: &PackageLineage, b: &PackageLineage) -> bool {
    a.commit == b.commit && a.remote_uri == b.remote_uri && a.base_hash == b.base_hash
}

/// Whether two entries track the same paths at the same rows. A row is the
/// path's hash; its timestamp moves on every re-install and says nothing.
pub(super) fn same_rows(a: &LineagePaths, b: &LineagePaths) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && va.hash == vb.hash)
}

/// The paths `current` tracks that `started` did not, or tracks at another
/// row: another writer placed them while this one worked.
pub(super) fn paths_added(started: &PackageLineage, current: &PackageLineage) -> Vec<PathBuf> {
    current
        .paths
        .iter()
        .filter(|(path, state)| {
            started
                .paths
                .get(*path)
                .is_none_or(|was| was.hash != state.hash)
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// The paths `started` tracked that `current` no longer does: uninstalled
/// while this writer worked.
pub(super) fn paths_removed(started: &PackageLineage, current: &PackageLineage) -> Vec<PathBuf> {
    started
        .paths
        .keys()
        .filter(|path| !current.paths.contains_key(*path))
        .cloned()
        .collect()
}

fn pick<T: PartialEq + Clone>(started: &T, next: &T, current: &T) -> T {
    if next == started {
        current.clone()
    } else {
        next.clone()
    }
}

/// `current` with this writer's change applied: every field, and every path,
/// the writer changed from `started` takes the writer's value; everything else
/// keeps its current one. So a field another writer moved meanwhile survives,
/// unless this writer changed it too, which the writer's rule has decided
/// before it gets here.
pub(super) fn own_change(
    started: &PackageLineage,
    next: &PackageLineage,
    current: &PackageLineage,
) -> PackageLineage {
    let keys: BTreeSet<&PathBuf> = started
        .paths
        .keys()
        .chain(next.paths.keys())
        .chain(current.paths.keys())
        .collect();
    let paths: LineagePaths = keys
        .into_iter()
        .filter_map(|path| {
            pick(
                &started.paths.get(path),
                &next.paths.get(path),
                &current.paths.get(path),
            )
            .map(|state| (path.clone(), state.clone()))
        })
        .collect();
    PackageLineage {
        commit: pick(&started.commit, &next.commit, &current.commit),
        remote_uri: pick(&started.remote_uri, &next.remote_uri, &current.remote_uri),
        base_hash: pick(&started.base_hash, &next.base_hash, &current.base_hash),
        latest_hash: pick(
            &started.latest_hash,
            &next.latest_hash,
            &current.latest_hash,
        ),
        paths,
        sync_scope: pick(&started.sync_scope, &next.sync_scope, &current.sync_scope),
    }
}

/// The paths `next` placed on disk that `started` did not have there: new, or
/// at another row.
pub(super) fn placed(started: &PackageLineage, next: &PackageLineage) -> LineagePaths {
    next.paths
        .iter()
        .filter(|(path, state)| {
            started
                .paths
                .get(*path)
                .is_none_or(|was| was.hash != state.hash)
        })
        .map(|(path, state)| (path.clone(), state.clone()))
        .collect()
}

/// A row that stands for "the bytes of `hash`", for the checks that take one.
pub(super) fn row_of(path: &Path, hash: &Multihash<256>) -> Option<ManifestRow> {
    Some(ManifestRow {
        logical_key: path.to_path_buf(),
        hash: ObjectHash::try_from(*hash).ok()?,
        ..ManifestRow::default()
    })
}

/// Whether the file at `file` still holds the bytes of `hash`. A file that is
/// missing or unreadable does not.
pub(super) async fn holds(storage: &impl Storage, file: &Path, hash: &Multihash<256>) -> bool {
    let Some(row) = row_of(file, hash) else {
        return false;
    };
    matches!(
        refresh_hash(storage, &file.to_path_buf(), row).await,
        Ok(None)
    )
}

/// Deletes each file that still holds the bytes a writer placed there. A file
/// someone edited since is left alone.
pub(super) async fn remove_if_unchanged(
    storage: &impl Storage,
    package_home: &Path,
    files: &BTreeMap<PathBuf, Multihash<256>>,
) {
    for (path, hash) in files {
        let file = package_home.join(path);
        if holds(storage, &file, hash).await {
            let _ = storage.remove_file(&file).await;
        }
    }
}

/// [`remove_if_unchanged`] over what a writer's entry says it placed.
pub(super) async fn remove_placed(
    storage: &impl Storage,
    package_home: &Path,
    placed: &LineagePaths,
) {
    let files = placed
        .iter()
        .map(|(path, state)| (path.clone(), state.hash))
        .collect();
    remove_if_unchanged(storage, package_home, &files).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lineage::CommitState;
    use crate::lineage::PathState;
    use crate::lineage::SyncScope;

    fn state(seed: u8) -> PathState {
        PathState {
            timestamp: chrono::DateTime::UNIX_EPOCH,
            hash: Multihash::wrap(0x12, &[seed; 32]).unwrap(),
        }
    }

    fn entry(base: &str, paths: &[(&str, u8)]) -> PackageLineage {
        PackageLineage {
            base_hash: base.to_string(),
            latest_hash: base.to_string(),
            paths: paths
                .iter()
                .map(|(p, seed)| (PathBuf::from(p), state(*seed)))
                .collect(),
            ..PackageLineage::default()
        }
    }

    /// Only the writer's own change lands; what another writer changed in the
    /// meantime survives.
    #[test]
    fn own_change_keeps_what_the_writer_left_alone() {
        let started = entry("r1", &[("a", 1)]);
        let next = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::DateTime::UNIX_EPOCH,
                hash: "c".to_string(),
                prev_hashes: Vec::new(),
            }),
            ..entry("r1", &[("a", 2)])
        };
        let current = PackageLineage {
            latest_hash: "r9".to_string(),
            sync_scope: SyncScope::EntirePackage,
            ..entry("r1", &[("a", 1), ("b", 3)])
        };

        let merged = own_change(&started, &next, &current);
        assert_eq!(merged.commit, next.commit);
        assert_eq!(merged.latest_hash, "r9");
        assert_eq!(merged.sync_scope, SyncScope::EntirePackage);
        assert_eq!(
            merged.paths,
            entry("", &[("a", 2), ("b", 3)]).paths,
            "the writer's row for a, and b as another writer placed it"
        );
    }

    /// A path the writer removed stays removed.
    #[test]
    fn own_change_applies_a_removal() {
        let started = entry("r1", &[("a", 1), ("b", 2)]);
        let next = entry("r1", &[("b", 2)]);
        let current = entry("r1", &[("a", 1), ("b", 2), ("c", 3)]);
        assert_eq!(
            own_change(&started, &next, &current).paths,
            entry("", &[("b", 2), ("c", 3)]).paths
        );
    }

    #[test]
    fn added_and_removed_paths_are_told_apart() {
        let started = entry("r1", &[("a", 1), ("b", 2)]);
        let current = entry("r1", &[("b", 9), ("c", 3)]);
        assert_eq!(
            paths_added(&started, &current),
            vec![PathBuf::from("b"), PathBuf::from("c")]
        );
        assert_eq!(paths_removed(&started, &current), vec![PathBuf::from("a")]);
    }
}
