//! A package's file entries, built from one status computation.
//!
//! Shared by v1's `get_installed_package_data` and v2's `get_package_page_data`,
//! so neither surface opens the other's module: each passes the status it
//! already computed, and the entries are classified by that one answer.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::quilt;

/// The most entries one read sends.
pub const ENTRIES_CAP: usize = 1000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageEntryData {
    pub filename: String,
    pub size: u64,
    pub status: String,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<String>,
    pub namespace: quilt_uri::Namespace,
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
    /// Matched by `.quiltignore` in the local walk.
    pub ignored: usize,
}

impl EntryCounts {
    fn of(entries: &[InstalledPackageEntryData]) -> Self {
        let mut counts = Self::default();
        for entry in entries {
            if entry.ignored_by.is_some() {
                counts.ignored += 1;
                continue;
            }
            counts.all += 1;
            match entry.status.as_str() {
                "added" | "modified" | "deleted" => counts.changed += 1,
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
/// `.quiltignore` matched. `records` are the manifest's rows, for their sizes.
pub fn entry_list(
    namespace: &quilt_uri::Namespace,
    status: &quilt::lineage::InstalledPackageStatus,
    installed_paths: &quilt::lineage::LineagePaths,
    records: &BTreeMap<PathBuf, quilt::manifest::ManifestRow>,
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
            namespace: namespace.clone(),
        });
    }
    for (filename, pattern, size) in &status.ignored_files {
        entries.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: *size,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: Some(pattern.clone()),
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
