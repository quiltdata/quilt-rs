use std::path::Path;
use std::path::PathBuf;

use super::Change;
use super::ChangeSet;
use super::PackageLineage;
use super::SyncScope;

/// How much of its current revision a copy holds, in files and in bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadCounts {
    /// The scope the copy keeps.
    pub scope: SyncScope,
    /// The files the current revision lists.
    pub listed: usize,
    /// The listed files still to download: see [`Self::is_outstanding`].
    pub outstanding: usize,
    /// The outstanding files the scope covers: see [`Self::in_backlog`].
    pub backlog: usize,
    /// Every deletion the status reports.
    pub deleted_here: usize,
    /// The bytes the manifest records for the same files; `None` when they do
    /// not add up in a `u64`.
    pub size: Option<DownloadSize>,
}

/// The manifest's recorded bytes of the listed files and of the outstanding
/// ones. Not disk usage: no file is stat'ed for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadSize {
    pub listed: u64,
    pub outstanding: u64,
    pub backlog: u64,
}

impl DownloadSize {
    /// The listed bytes that are not outstanding. A file deleted here is not
    /// outstanding, so it counts.
    #[must_use]
    pub fn downloaded(&self) -> u64 {
        self.listed - self.outstanding
    }
}

impl DownloadCounts {
    /// Counts `rows`, the logical keys of the current revision's rows with the
    /// sizes the rows record.
    #[must_use]
    pub fn count<'a>(
        lineage: &PackageLineage,
        rows: impl IntoIterator<Item = (&'a PathBuf, u64)>,
        changes: &ChangeSet,
    ) -> Self {
        let mut listed = 0;
        let mut outstanding = 0;
        let mut backlog = 0;
        let mut size = Some(DownloadSize::default());
        for (key, bytes) in rows {
            let missing = Self::is_outstanding(lineage, changes, key);
            let wanted = missing && lineage.sync_scope.covers_untracked(key);
            listed += 1;
            outstanding += usize::from(missing);
            backlog += usize::from(wanted);
            // `outstanding` and `backlog` sum subsets of `listed`, so they fit
            // whenever `listed` does.
            size = size.and_then(|size| {
                Some(DownloadSize {
                    listed: size.listed.checked_add(bytes)?,
                    outstanding: size.outstanding + if missing { bytes } else { 0 },
                    backlog: size.backlog + if wanted { bytes } else { 0 },
                })
            });
        }
        Self {
            scope: lineage.sync_scope.clone(),
            listed,
            outstanding,
            backlog,
            deleted_here: changes
                .values()
                .filter(|change| matches!(change, Change::Removed(_)))
                .count(),
            size,
        }
    }

    /// Whether the revision's file at `key` is still to download: this copy
    /// does not track it and has no local change there. A file deleted here
    /// is a change, so it is not outstanding.
    #[must_use]
    pub fn is_outstanding(lineage: &PackageLineage, changes: &ChangeSet, key: &Path) -> bool {
        !lineage.paths.contains_key(key) && !changes.contains_key(key)
    }

    /// Whether the revision's file at `key` is outstanding and the scope
    /// covers it: what downloading the backlog fetches. Never true under
    /// individual-file scope, where nothing waits for a download.
    #[must_use]
    pub fn in_backlog(lineage: &PackageLineage, changes: &ChangeSet, key: &Path) -> bool {
        Self::is_outstanding(lineage, changes, key) && lineage.sync_scope.covers_untracked(key)
    }

    /// The listed files that are not outstanding.
    #[must_use]
    pub fn downloaded(&self) -> usize {
        self.listed - self.outstanding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lineage::Change;
    use crate::lineage::PathState;
    use crate::manifest::ManifestRow;

    fn tracking(paths: &[&str], scope: SyncScope) -> PackageLineage {
        PackageLineage {
            paths: paths
                .iter()
                .map(|path| (PathBuf::from(path), PathState::default()))
                .collect(),
            sync_scope: scope,
            ..PackageLineage::default()
        }
    }

    /// The listed paths, each 7 bytes.
    fn rows(paths: &[&str]) -> Vec<(PathBuf, u64)> {
        paths.iter().map(|path| (PathBuf::from(path), 7)).collect()
    }

    fn count(
        lineage: &PackageLineage,
        rows: &[(PathBuf, u64)],
        changes: &ChangeSet,
    ) -> DownloadCounts {
        DownloadCounts::count(
            lineage,
            rows.iter().map(|(key, size)| (key, *size)),
            changes,
        )
    }

    fn changed(path: &str, change: fn(ManifestRow) -> Change) -> ChangeSet {
        ChangeSet::from([(PathBuf::from(path), change(ManifestRow::default()))])
    }

    #[test]
    fn a_whole_package_with_every_file_here_has_nothing_outstanding() {
        let lineage = tracking(&["a.csv", "b.csv"], SyncScope::EntirePackage);

        let counts = count(&lineage, &rows(&["a.csv", "b.csv"]), &ChangeSet::new());

        assert_eq!(
            counts,
            DownloadCounts {
                scope: SyncScope::EntirePackage,
                listed: 2,
                outstanding: 0,
                backlog: 0,
                deleted_here: 0,
                size: Some(DownloadSize {
                    listed: 14,
                    outstanding: 0,
                    backlog: 0,
                }),
            }
        );
    }

    #[test]
    fn a_whole_package_counts_the_files_it_does_not_track() {
        let lineage = tracking(&["a.csv"], SyncScope::EntirePackage);

        let counts = count(
            &lineage,
            &rows(&["a.csv", "b.csv", "c.csv"]),
            &ChangeSet::new(),
        );

        assert_eq!(counts.listed, 3);
        assert_eq!(counts.outstanding, 2);
        assert_eq!(counts.backlog, 2);
        assert_eq!(counts.downloaded(), 1);
    }

    #[test]
    fn kept_folders_count_only_their_files_as_the_backlog() {
        let lineage = tracking(&["top.csv"], SyncScope::folders(["data/raw"]));

        let counts = count(
            &lineage,
            &rows(&[
                "top.csv",
                "data/raw/a.csv",
                "data/raw/x/b.csv",
                "data/raw2/c.csv",
            ]),
            &ChangeSet::new(),
        );

        assert_eq!(
            (counts.listed, counts.outstanding, counts.backlog),
            (4, 3, 2)
        );
        assert_eq!(
            counts.size,
            Some(DownloadSize {
                listed: 28,
                outstanding: 21,
                backlog: 14,
            })
        );
    }

    #[test]
    fn picked_files_have_no_backlog() {
        let lineage = tracking(&["a.csv"], SyncScope::IndividualFiles);
        let changes = ChangeSet::new();

        let counts = count(&lineage, &rows(&["a.csv", "b.csv"]), &changes);

        assert_eq!((counts.outstanding, counts.backlog), (1, 0));
        assert!(!DownloadCounts::in_backlog(
            &lineage,
            &changes,
            Path::new("b.csv")
        ));
    }

    #[test]
    fn picked_files_count_what_is_here_against_the_revision() {
        let lineage = tracking(&["a.csv"], SyncScope::IndividualFiles);
        let partial = count(
            &lineage,
            &rows(&["a.csv", "b.csv", "c.csv"]),
            &ChangeSet::new(),
        );
        assert_eq!(partial.scope, SyncScope::IndividualFiles);
        assert_eq!((partial.downloaded(), partial.listed), (1, 3));

        let full = count(&lineage, &rows(&["a.csv"]), &ChangeSet::new());
        assert_eq!((full.downloaded(), full.listed), (1, 1));
    }

    #[test]
    fn a_locally_changed_file_is_not_outstanding() {
        let lineage = tracking(&[], SyncScope::EntirePackage);
        let listed = rows(&["a.csv", "b.csv"]);

        let modified = count(&lineage, &listed, &changed("a.csv", Change::Modified));
        assert_eq!(modified.outstanding, 1);

        let removed = count(&lineage, &listed, &changed("b.csv", Change::Removed));
        assert_eq!(removed.outstanding, 1);
    }

    #[test]
    fn a_tracked_file_the_revision_no_longer_lists_is_not_counted() {
        let lineage = tracking(&["a.csv", "z.csv"], SyncScope::EntirePackage);

        let counts = count(&lineage, &rows(&["a.csv"]), &ChangeSet::new());

        assert_eq!((counts.listed, counts.outstanding), (1, 0));
    }

    #[test]
    fn a_package_whose_scope_was_never_set_keeps_individual_files() {
        let lineage: PackageLineage =
            serde_json::from_str(r#"{"commit":null,"base_hash":"","latest_hash":"","paths":{}}"#)
                .expect("a lineage with no scope");

        let counts = count(&lineage, &rows(&["a.csv"]), &ChangeSet::new());

        assert_eq!(counts.scope, SyncScope::IndividualFiles);
        assert_eq!((counts.downloaded(), counts.listed), (0, 1));
    }

    fn sized(rows: &[(&str, u64)]) -> Vec<(PathBuf, u64)> {
        rows.iter()
            .map(|(path, size)| (PathBuf::from(path), *size))
            .collect()
    }

    /// The outstanding bytes are the backlog's; a file changed or deleted here
    /// counts as downloaded.
    #[test]
    fn the_size_leaves_the_backlog_out_of_downloaded() {
        let lineage = tracking(&["a.csv"], SyncScope::EntirePackage);
        let mut changes = changed("b.csv", Change::Modified);
        changes.extend(changed("c.csv", Change::Removed));

        let counts = count(
            &lineage,
            &sized(&[
                ("a.csv", 1),
                ("b.csv", 20),
                ("c.csv", 300),
                ("d.csv", 4_000),
            ]),
            &changes,
        );

        assert_eq!(
            counts.size,
            Some(DownloadSize {
                listed: 4_321,
                outstanding: 4_000,
                backlog: 4_000,
            })
        );
        assert_eq!(counts.size.unwrap().downloaded(), 321);
    }

    #[test]
    fn deleted_here_is_every_removal_the_status_reports() {
        let lineage = tracking(&["a.csv", "b.csv"], SyncScope::EntirePackage);
        let listed = rows(&["a.csv", "b.csv"]);
        let mut changes = changed("a.csv", Change::Removed);
        changes.extend(changed("b.csv", Change::Removed));
        changes.extend(changed("c.csv", Change::Modified));

        assert_eq!(count(&lineage, &listed, &changes).deleted_here, 2);
        assert_eq!(count(&lineage, &listed, &ChangeSet::new()).deleted_here, 0);
    }

    /// Sizes that do not add up in a `u64` are not read, rather than wrapped
    /// into a figure nobody has: the file counts still are.
    #[test]
    fn a_size_past_u64_is_not_read_and_the_counts_still_are() {
        let lineage = tracking(&[], SyncScope::EntirePackage);

        let counts = count(
            &lineage,
            &sized(&[("a.csv", u64::MAX), ("b.csv", 1)]),
            &ChangeSet::new(),
        );

        assert_eq!(counts.size, None);
        assert_eq!((counts.listed, counts.outstanding), (2, 2));
    }

    #[test]
    fn an_empty_revision_weighs_nothing() {
        let counts = count(
            &tracking(&[], SyncScope::EntirePackage),
            &[],
            &ChangeSet::new(),
        );

        assert_eq!(counts.size, Some(DownloadSize::default()));
    }
}
