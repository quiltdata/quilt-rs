//! The pane's selection and its one action, `[Download]`.
//!
//! Only a not-downloaded, non-ignored row can be ticked (design §3, and the
//! ruling that an ignored file is stopped being ignored before it is fetched).
//! What is ticked is kept as paths and always read through the rows the list
//! offers, so a path that was downloaded, or is no longer listed, drops out of
//! every count without anyone clearing it
//! (`quiltsync-keep-file-selection#d-derive-by-intersection`).

use std::collections::BTreeSet;

use leptos::prelude::*;

use crate::commands::{KeepingData, KeepingScope};
use crate::kit::CheckState;

/// What the Keeping scope fetches by itself, so no box offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// `The whole package`: every file.
    Package,
    /// Every file under these folders. None under individual-file Keeping.
    Folders(Vec<String>),
}

impl Kept {
    /// The scope as the page read it.
    #[must_use]
    pub fn of(keeping: &KeepingData) -> Self {
        match keeping.scope {
            KeepingScope::EntirePackage => Self::Package,
            KeepingScope::Folders => Self::Folders(keeping.folders.clone()),
            KeepingScope::IndividualFiles => Self::Folders(Vec::new()),
        }
    }

    /// Whether the scope fetches `path`. A folder covers the paths under it
    /// on a folder boundary: `data/raw` covers `data/raw/x/b.csv`, not
    /// `data/raw2/a.csv`.
    #[must_use]
    pub fn fetches(&self, path: &str) -> bool {
        match self {
            Self::Package => true,
            // Either separator, as the library reads a path.
            Self::Folders(folders) => {
                let path = path.replace('\\', "/");
                folders.iter().any(|folder| {
                    path.strip_prefix(folder.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
            }
        }
    }
}

/// The pane's selection and what it feeds. The page owns the signals, so a
/// re-read — the watcher's news while the download writes files — neither
/// clears the ticks nor drops the button's spinner.
#[derive(Clone, Copy)]
pub struct Picking {
    /// The ticked paths. Read only through the rows on offer; see the module doc.
    pub ticked: RwSignal<BTreeSet<String>>,
    /// The footer's download is running.
    pub downloading: Signal<bool>,
    /// The page's one-command-at-a-time lock.
    pub busy: Signal<bool>,
    /// `[Download]`: the ticked paths, in path order.
    pub on_download: Callback<Vec<String>>,
    /// What Keeping fetches by itself: its rows get no box, and neither
    /// select-all nor the footer counts them.
    pub kept: StoredValue<Kept>,
}

/// Nothing ticked, nothing running, and a download that goes nowhere — what
/// a pane drawn without a page (a test, a scene) selects with.
impl Default for Picking {
    fn default() -> Self {
        Self {
            ticked: RwSignal::new(BTreeSet::new()),
            downloading: Signal::stored(false),
            busy: Signal::stored(false),
            on_download: Callback::new(|_| ()),
            kept: StoredValue::new(Kept::Folders(Vec::new())),
        }
    }
}

/// The ticked paths among `offered`, in `offered`'s order — path order, since
/// the list is.
#[must_use]
pub fn ticked_among(ticked: &BTreeSet<String>, offered: &[String]) -> Vec<String> {
    offered
        .iter()
        .filter(|p| ticked.contains(*p))
        .cloned()
        .collect()
}

/// What select-all or a heading's box does: tick every offered path, or clear
/// them. Paths outside `offered` keep their ticks.
pub fn tick_all(ticked: &mut BTreeSet<String>, offered: &[String], next: bool) {
    for path in offered {
        if next {
            ticked.insert(path.clone());
        } else {
            ticked.remove(path);
        }
    }
}

/// A heading's box, from the rows under it that can be ticked.
#[must_use]
pub fn group_state(ticked: &BTreeSet<String>, members: &[String]) -> CheckState {
    match ticked_among(ticked, members).len() {
        0 => CheckState::Off,
        n if n == members.len() => CheckState::On,
        _ => CheckState::Mixed,
    }
}

/// The band's words when the download left files behind: the lead, and the
/// paths as the detail. `None` when every file came down.
#[must_use]
pub fn unavailable(asked: usize, skipped: &[String]) -> Option<(String, String)> {
    let left = skipped.len();
    if left == 0 {
        return None;
    }
    let files = if left == 1 {
        "1 file is"
    } else {
        &format!("{left} files are")
    };
    Some((
        format!(
            "Downloaded {} of {asked}. {files} no longer on the remote at this revision.",
            asked.saturating_sub(left)
        ),
        skipped.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(ps: &[&str]) -> Vec<String> {
        ps.iter().map(ToString::to_string).collect()
    }

    fn set(ps: &[&str]) -> BTreeSet<String> {
        ps.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_kept_folder_covers_the_paths_under_it_on_a_folder_boundary() {
        let kept = Kept::Folders(paths(&["data/raw"]));
        assert!(kept.fetches("data/raw/a.csv"));
        assert!(kept.fetches("data/raw/x/b.csv"));
        assert!(!kept.fetches("data/raw2/a.csv"));
        assert!(!kept.fetches("data/raw"));
        assert!(!kept.fetches("notes.txt"));
        assert!(kept.fetches("data\\raw\\a.csv"), "a Windows path");
        assert!(!Kept::Folders(Vec::new()).fetches("data/raw/a.csv"));
        assert!(Kept::Package.fetches("notes.txt"));
    }

    #[test]
    fn only_offered_ticks_count_and_they_keep_the_lists_order() {
        // `gone.csv` was downloaded since it was ticked: it is no longer offered.
        let ticked = set(&["c.csv", "a.csv", "gone.csv"]);
        assert_eq!(
            ticked_among(&ticked, &paths(&["a.csv", "b.csv", "c.csv"])),
            paths(&["a.csv", "c.csv"])
        );
    }

    #[test]
    fn select_all_ticks_and_clears_only_what_is_offered() {
        let mut ticked = set(&["hidden.csv"]);
        tick_all(&mut ticked, &paths(&["a.csv", "b.csv"]), true);
        assert_eq!(ticked, set(&["a.csv", "b.csv", "hidden.csv"]));
        tick_all(&mut ticked, &paths(&["a.csv", "b.csv"]), false);
        assert_eq!(ticked, set(&["hidden.csv"]));
    }

    #[test]
    fn a_heading_box_is_off_mixed_or_on_by_its_own_rows() {
        let members = paths(&["raw/a.csv", "raw/b.csv"]);
        assert_eq!(group_state(&set(&[]), &members), CheckState::Off);
        assert_eq!(
            group_state(&set(&["raw/a.csv"]), &members),
            CheckState::Mixed
        );
        assert_eq!(
            group_state(&set(&["raw/a.csv", "raw/b.csv"]), &members),
            CheckState::On
        );
        // A tick elsewhere says nothing about this group.
        assert_eq!(
            group_state(&set(&["notes/x.md"]), &members),
            CheckState::Off
        );
    }

    #[test]
    fn a_whole_download_says_nothing() {
        assert_eq!(unavailable(17, &[]), None);
    }

    /// The rulings' own example.
    #[test]
    fn a_download_that_left_files_behind_counts_them_and_names_them() {
        let skipped = paths(&["raw/a.csv", "raw/b.csv", "raw/c.csv"]);
        assert_eq!(
            unavailable(17, &skipped),
            Some((
                "Downloaded 14 of 17. 3 files are no longer on the remote at this revision."
                    .to_string(),
                "raw/a.csv, raw/b.csv, raw/c.csv".to_string()
            ))
        );
    }

    #[test]
    fn one_file_left_behind_is_singular() {
        assert_eq!(
            unavailable(2, &paths(&["raw/a.csv"])).map(|(lead, _)| lead),
            Some(
                "Downloaded 1 of 2. 1 file is no longer on the remote at this revision."
                    .to_string()
            )
        );
    }
}
