use std::path::Path;
use std::path::PathBuf;

use super::ChangeSet;
use super::PackageLineage;
use super::SyncScope;

/// How much of its current revision a copy holds, counted in files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadCounts {
    /// The scope the copy keeps.
    pub scope: SyncScope,
    /// The files the current revision lists.
    pub listed: usize,
    /// The listed files still to download: see [`Self::is_outstanding`].
    pub outstanding: usize,
}

impl DownloadCounts {
    /// Counts `keys`, the logical keys of the current revision's rows.
    #[must_use]
    pub fn count<'a>(
        lineage: &PackageLineage,
        keys: impl IntoIterator<Item = &'a PathBuf>,
        changes: &ChangeSet,
    ) -> Self {
        let (listed, outstanding) = keys.into_iter().fold((0, 0), |(listed, outstanding), key| {
            let missing = Self::is_outstanding(lineage, changes, key);
            (listed + 1, outstanding + usize::from(missing))
        });
        Self {
            scope: lineage.sync_scope,
            listed,
            outstanding,
        }
    }

    /// Whether the revision's file at `key` is still to download: this copy
    /// does not track it and has no local change there. A file deleted here
    /// is a change, so it is not outstanding.
    #[must_use]
    pub fn is_outstanding(lineage: &PackageLineage, changes: &ChangeSet, key: &Path) -> bool {
        !lineage.paths.contains_key(key) && !changes.contains_key(key)
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

    fn keys(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn changed(path: &str, change: fn(ManifestRow) -> Change) -> ChangeSet {
        ChangeSet::from([(PathBuf::from(path), change(ManifestRow::default()))])
    }

    #[test]
    fn a_whole_package_with_every_file_here_has_nothing_outstanding() {
        let lineage = tracking(&["a.csv", "b.csv"], SyncScope::EntirePackage);

        let counts = DownloadCounts::count(&lineage, &keys(&["a.csv", "b.csv"]), &ChangeSet::new());

        assert_eq!(
            counts,
            DownloadCounts {
                scope: SyncScope::EntirePackage,
                listed: 2,
                outstanding: 0,
            }
        );
    }

    #[test]
    fn a_whole_package_counts_the_files_it_does_not_track() {
        let lineage = tracking(&["a.csv"], SyncScope::EntirePackage);

        let counts = DownloadCounts::count(
            &lineage,
            &keys(&["a.csv", "b.csv", "c.csv"]),
            &ChangeSet::new(),
        );

        assert_eq!(counts.listed, 3);
        assert_eq!(counts.outstanding, 2);
        assert_eq!(counts.downloaded(), 1);
    }

    #[test]
    fn picked_files_count_what_is_here_against_the_revision() {
        let lineage = tracking(&["a.csv"], SyncScope::IndividualFiles);
        let partial = DownloadCounts::count(
            &lineage,
            &keys(&["a.csv", "b.csv", "c.csv"]),
            &ChangeSet::new(),
        );
        assert_eq!(partial.scope, SyncScope::IndividualFiles);
        assert_eq!((partial.downloaded(), partial.listed), (1, 3));

        let full = DownloadCounts::count(&lineage, &keys(&["a.csv"]), &ChangeSet::new());
        assert_eq!((full.downloaded(), full.listed), (1, 1));
    }

    #[test]
    fn a_locally_changed_file_is_not_outstanding() {
        let lineage = tracking(&[], SyncScope::EntirePackage);
        let listed = keys(&["a.csv", "b.csv"]);

        let modified =
            DownloadCounts::count(&lineage, &listed, &changed("a.csv", Change::Modified));
        assert_eq!(modified.outstanding, 1);

        let removed = DownloadCounts::count(&lineage, &listed, &changed("b.csv", Change::Removed));
        assert_eq!(removed.outstanding, 1);
    }

    #[test]
    fn a_tracked_file_the_revision_no_longer_lists_is_not_counted() {
        let lineage = tracking(&["a.csv", "z.csv"], SyncScope::EntirePackage);

        let counts = DownloadCounts::count(&lineage, &keys(&["a.csv"]), &ChangeSet::new());

        assert_eq!((counts.listed, counts.outstanding), (1, 0));
    }

    #[test]
    fn a_package_whose_scope_was_never_set_keeps_individual_files() {
        let lineage: PackageLineage =
            serde_json::from_str(r#"{"commit":null,"base_hash":"","latest_hash":"","paths":{}}"#)
                .expect("a lineage with no scope");

        let counts = DownloadCounts::count(&lineage, &keys(&["a.csv"]), &ChangeSet::new());

        assert_eq!(counts.scope, SyncScope::IndividualFiles);
        assert_eq!((counts.downloaded(), counts.listed), (0, 1));
    }
}
