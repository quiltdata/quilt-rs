//! A package's file entries, built from one status computation.
//!
//! Shared by v1's `get_installed_package_data` and v2's `get_package_page_data`,
//! so neither surface opens the other's module: each passes the status it
//! already computed, and the entries are classified by that one answer.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::engine::quilt;

/// The most entries one read sends.
pub const ENTRIES_CAP: usize = 1000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageEntryData {
    pub filename: String,
    pub size: u64,
    pub status: String,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<IgnoredByData>,
    /// An ignored folder, not a file: `filename` is its path without a
    /// trailing slash, and `size` is 0 because the walk never enters it.
    pub folder: bool,
    pub namespace: quilt_uri::Namespace,
}

/// The rule that ignored an entry: its line as written, and where it came from.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct IgnoredByData {
    pub pattern: String,
    pub source: RuleSourceData,
}

/// Where an ignore rule came from. Lines are 1-based.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuleSourceData {
    Default,
    User { line: u64 },
    Package { line: u64 },
}

impl From<&quilt::quiltignore::IgnoredBy> for IgnoredByData {
    fn from(by: &quilt::quiltignore::IgnoredBy) -> Self {
        use quilt::quiltignore::RuleSource;
        Self {
            pattern: by.pattern.clone(),
            source: match by.source {
                RuleSource::Default => RuleSourceData::Default,
                RuleSource::User { line, .. } => RuleSourceData::User { line },
                RuleSource::Package { line } => RuleSourceData::Package { line },
            },
        }
    }
}

/// The v2 file pane's facet counts, over the whole package rather than the
/// capped entries. The facets are disjoint apart from `all`, which holds every
/// entry except the ignored ones.
#[derive(Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntryCounts {
    pub all: usize,
    /// Added, modified and deleted.
    pub changed: usize,
    /// Listed by the manifest, not in this copy: the `remote` status.
    pub not_downloaded: usize,
    /// Matched by `.quiltignore` in the local walk: files and folders.
    pub ignored: usize,
    /// Also inside `ignored`: the folders among them.
    pub ignored_folders: usize,
    /// Also inside `changed`; on its own because the v2 pane's all-downloaded
    /// caption must know about a deleted file past the list cap.
    pub deleted: usize,
}

impl EntryCounts {
    fn of(entries: &[InstalledPackageEntryData]) -> Self {
        let mut counts = Self::default();
        for entry in entries {
            if entry.ignored_by.is_some() {
                counts.ignored += 1;
                if entry.folder {
                    counts.ignored_folders += 1;
                }
                continue;
            }
            counts.all += 1;
            match entry.status.as_str() {
                "added" | "modified" => counts.changed += 1,
                "deleted" => {
                    counts.changed += 1;
                    counts.deleted += 1;
                }
                "remote" => counts.not_downloaded += 1,
                _ => {}
            }
        }
        counts
    }
}

/// The package's entries, sorted by path and capped, with the whole-package
/// facts the cap would otherwise hide.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryList {
    /// Sorted by path, then capped at [`ENTRIES_CAP`].
    pub entries: Vec<InstalledPackageEntryData>,
    /// Whole-package facet counts; `counts.all + counts.ignored == total`.
    pub counts: EntryCounts,
    /// Every entry the package has, ignored ones included, before the cap.
    pub total: usize,
    /// Whether the cap dropped entries: `total > entries.len()`. Set here so
    /// the UI never infers truncation from a length.
    pub truncated: bool,
}

/// Classify every file the package has by `status`: its changes, the tracked
/// paths it leaves pristine, the listed paths this copy lacks, and the files
/// and folders `.quiltignore` matched. `records` are the manifest's rows, for
/// their sizes. v1 lists no ignored folders (`with_ignored_folders` false), so
/// they never take its rows' place under the cap.
pub fn entry_list(
    namespace: &quilt_uri::Namespace,
    status: &quilt::lineage::InstalledPackageStatus,
    installed_paths: &quilt::lineage::LineagePaths,
    records: &BTreeMap<PathBuf, quilt::manifest::ManifestRow>,
    with_ignored_folders: bool,
) -> EntryList {
    let modified_entries = &status.changes;
    let junky_map: HashMap<_, _> = status
        .junky_changes
        .iter()
        .map(|(p, pat)| (p.clone(), pat.clone()))
        .collect();

    let mut entries = Vec::new();
    for (filename, change) in modified_entries {
        let (status_str, size) = match change {
            quilt::lineage::Change::Added(r) => ("added", r.size),
            quilt::lineage::Change::Modified(r) => ("modified", r.size),
            quilt::lineage::Change::Removed(r) => ("deleted", r.size),
        };
        entries.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size,
            status: status_str.to_string(),
            junky_pattern: junky_map.get(filename).cloned(),
            ignored_by: None,
            folder: false,
            namespace: namespace.clone(),
        });
    }
    for filename in installed_paths.keys() {
        if modified_entries.contains_key(filename) {
            continue;
        }
        if let Some(row) = records.get(filename) {
            entries.push(InstalledPackageEntryData {
                filename: filename.display().to_string(),
                size: row.size,
                status: "pristine".to_string(),
                junky_pattern: None,
                ignored_by: None,
                folder: false,
                namespace: namespace.clone(),
            });
        }
    }
    for (filename, row) in records {
        if installed_paths.contains_key(filename) || modified_entries.contains_key(filename) {
            continue;
        }
        entries.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: row.size,
            status: "remote".to_string(),
            junky_pattern: None,
            ignored_by: None,
            folder: false,
            namespace: namespace.clone(),
        });
    }
    for (filename, ignored_by, size) in &status.ignored_files {
        entries.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: *size,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: Some(ignored_by.into()),
            folder: false,
            namespace: namespace.clone(),
        });
    }
    let ignored_dirs: &[_] = if with_ignored_folders {
        &status.ignored_dirs
    } else {
        &[]
    };
    for (path, ignored_by) in ignored_dirs {
        entries.push(InstalledPackageEntryData {
            filename: path.display().to_string(),
            size: 0,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: Some(ignored_by.into()),
            folder: true,
            namespace: namespace.clone(),
        });
    }

    // Sort every entry by path before capping, so the loaded rows are the
    // first paths rather than whichever change class filled the list first.
    entries.sort_by(|a, b| a.filename.cmp(&b.filename));
    let counts = EntryCounts::of(&entries);
    let total = entries.len();
    entries.truncate(ENTRIES_CAP);
    let truncated = total > entries.len();

    EntryList {
        entries,
        counts,
        total,
        truncated,
    }
}

/// [`entry_list`]'s counts without its rows: what a read needs when it shows
/// none of them, as the commit page's totals. Classifies every file as
/// `entry_list` does, so the two cannot disagree; a test holds them together.
pub fn entry_counts(
    status: &quilt::lineage::InstalledPackageStatus,
    installed_paths: &quilt::lineage::LineagePaths,
    records: &BTreeMap<PathBuf, quilt::manifest::ManifestRow>,
) -> EntryCounts {
    let changes = &status.changes;
    let mut counts = EntryCounts::default();
    for change in changes.values() {
        counts.all += 1;
        counts.changed += 1;
        if matches!(change, quilt::lineage::Change::Removed(_)) {
            counts.deleted += 1;
        }
    }
    // Pristine: tracked, unchanged, and listed by the manifest.
    counts.all += installed_paths
        .keys()
        .filter(|p| !changes.contains_key(*p) && records.contains_key(*p))
        .count();
    let not_downloaded = records
        .keys()
        .filter(|p| !installed_paths.contains_key(*p) && !changes.contains_key(*p))
        .count();
    counts.all += not_downloaded;
    counts.not_downloaded = not_downloaded;
    counts.ignored_folders = status.ignored_dirs.len();
    counts.ignored = status.ignored_files.len() + counts.ignored_folders;
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind of file at once: the count-only path and the list agree.
    #[test]
    fn entry_counts_match_the_list_s_counts() {
        let row = || quilt::manifest::ManifestRow::default();
        let mut status = quilt::lineage::InstalledPackageStatus::default();
        status.changes.insert(
            PathBuf::from("new.csv"),
            quilt::lineage::Change::Added(row()),
        );
        status.changes.insert(
            PathBuf::from("edited.csv"),
            quilt::lineage::Change::Modified(row()),
        );
        status.changes.insert(
            PathBuf::from("gone.csv"),
            quilt::lineage::Change::Removed(row()),
        );
        status.ignored_files.push((
            PathBuf::from("x.tmp"),
            quilt::quiltignore::IgnoredBy {
                pattern: "*.tmp".to_string(),
                source: quilt::quiltignore::RuleSource::Default,
            },
            1,
        ));
        status.ignored_dirs.push((
            PathBuf::from("cache"),
            quilt::quiltignore::IgnoredBy {
                pattern: "cache/".to_string(),
                source: quilt::quiltignore::RuleSource::Package { line: 1 },
            },
        ));
        let installed: quilt::lineage::LineagePaths =
            ["edited.csv", "gone.csv", "kept.csv", "stray.csv"]
                .into_iter()
                .map(|p| (PathBuf::from(p), quilt::lineage::PathState::default()))
                .collect();
        let records: BTreeMap<_, _> = ["edited.csv", "gone.csv", "kept.csv", "remote.csv"]
            .into_iter()
            .map(|p| (PathBuf::from(p), row()))
            .collect();
        let namespace = quilt_uri::Namespace::try_from("org/pkg").unwrap();

        let listed = entry_list(&namespace, &status, &installed, &records, true).counts;
        assert_eq!(entry_counts(&status, &installed, &records), listed);
        assert_eq!(
            listed,
            EntryCounts {
                all: 5,
                changed: 3,
                not_downloaded: 1,
                ignored: 2,
                ignored_folders: 1,
                deleted: 1,
            }
        );
    }

    /// v1 lists no ignored folders, so a package with a cap's worth of them
    /// still shows its files there, and v2 lists them all.
    #[test]
    fn ignored_folders_take_no_place_in_v1_s_list() {
        let mut status = quilt::lineage::InstalledPackageStatus::default();
        for i in 0..ENTRIES_CAP {
            status.ignored_dirs.push((
                PathBuf::from(format!("a{i:04}")),
                quilt::quiltignore::IgnoredBy {
                    pattern: "a*/".to_string(),
                    source: quilt::quiltignore::RuleSource::Package { line: 1 },
                },
            ));
        }
        let installed: quilt::lineage::LineagePaths =
            [(PathBuf::from("z.csv"), quilt::lineage::PathState::default())]
                .into_iter()
                .collect();
        let records: BTreeMap<_, _> = [(
            PathBuf::from("z.csv"),
            quilt::manifest::ManifestRow::default(),
        )]
        .into_iter()
        .collect();
        let namespace = quilt_uri::Namespace::try_from("org/pkg").unwrap();

        let v1 = entry_list(&namespace, &status, &installed, &records, false);
        let names: Vec<&str> = v1.entries.iter().map(|e| e.filename.as_str()).collect();
        assert_eq!(names, ["z.csv"]);
        assert_eq!((v1.total, v1.truncated), (1, false));
        assert_eq!(v1.counts.ignored_folders, 0);

        let v2 = entry_list(&namespace, &status, &installed, &records, true);
        assert_eq!((v2.total, v2.truncated), (ENTRIES_CAP + 1, true));
        assert_eq!(v2.counts.ignored_folders, ENTRIES_CAP);
    }

    /// Each ignored entry carries its rule's source, as the UI reads it; the
    /// user-wide file's path stays behind, since the row names it in words.
    #[test]
    fn ignored_entries_carry_the_rule_s_source() {
        use quilt::quiltignore::{IgnoredBy, RuleSource};
        let mut status = quilt::lineage::InstalledPackageStatus::default();
        status.ignored_files.push((
            PathBuf::from(".DS_Store"),
            IgnoredBy {
                pattern: ".DS_Store".to_string(),
                source: RuleSource::Default,
            },
            1,
        ));
        status.ignored_files.push((
            PathBuf::from("a.tmp"),
            IgnoredBy {
                pattern: "*.tmp".to_string(),
                source: RuleSource::User {
                    path: PathBuf::from("/domain/.quiltignore"),
                    line: 4,
                },
            },
            1,
        ));
        status.ignored_dirs.push((
            PathBuf::from("cache"),
            IgnoredBy {
                pattern: "cache/".to_string(),
                source: RuleSource::Package { line: 2 },
            },
        ));
        let namespace = quilt_uri::Namespace::try_from("org/pkg").unwrap();

        let list = entry_list(
            &namespace,
            &status,
            &quilt::lineage::LineagePaths::default(),
            &BTreeMap::new(),
            true,
        );
        let sources: Vec<_> = list
            .entries
            .iter()
            .map(|e| serde_json::to_value(&e.ignored_by).unwrap())
            .collect();
        assert_eq!(
            sources,
            [
                serde_json::json!({ "pattern": ".DS_Store", "source": { "kind": "default" } }),
                serde_json::json!({ "pattern": "*.tmp", "source": { "kind": "user", "line": 4 } }),
                serde_json::json!({ "pattern": "cache/", "source": { "kind": "package", "line": 2 } }),
            ]
        );
    }
}
