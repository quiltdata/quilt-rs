//! The wording of a pull's [report](quilt_rs::flow::PullReport), as a toast.
//!
//! Composed here rather than in the client because the groups are the report's
//! own: the stack renders what it is given, so the copy lives beside the tick
//! that posts it and nothing in the UI needs to know a revision exists.

use quilt_rs::flow::PullReport;
use quilt_uri::Namespace;
use std::path::PathBuf;

use crate::toast::ToastGroup;
use crate::toast::ToastKind;

/// How many paths a group names before it counts the rest.
///
/// A toast has room for a short list, and the point is that the user can see
/// *which* files rather than only how many — so it names some and is honest
/// about the remainder instead of truncating silently.
const NAMED: usize = 3;

/// A toast's parts, ready for [`ToastCenter::post`](crate::toast::ToastCenter::post).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportToast {
    pub kind: ToastKind,
    pub title: String,
    /// The lead sentence — what happened. Without it the toast opens with a
    /// package name and a file count and never says why it is on screen.
    pub body: String,
    pub groups: Vec<ToastGroup>,
}

/// One group: a heading that counts, and up to [`NAMED`] paths as data.
fn group(heading: &str, paths: &[PathBuf]) -> Option<ToastGroup> {
    if paths.is_empty() {
        return None;
    }
    let plural = if paths.len() == 1 { "" } else { "s" };
    Some(ToastGroup {
        heading: format!("{} file{plural} {heading}", paths.len()),
        items: paths
            .iter()
            .take(NAMED)
            .map(|p| p.display().to_string())
            .collect(),
        more: paths.len().saturating_sub(NAMED),
    })
}

/// The toast for a pull, or `None` when there is nothing to say.
///
/// `None` covers the metadata-only revision: its hashes advance and no file
/// moves, and in a stack that stands until dismissed an entry saying nothing
/// about files is one the user must clear by hand — which is what teaches
/// people to dismiss without reading.
///
/// The revision's own message is deliberately absent. It belongs to the package
/// screen and the CLI, where it does not crowd out the filenames; and a pull
/// spans revisions, so the message in hand is only the newest one's.
pub(crate) fn report_toast(namespace: &Namespace, report: &PullReport) -> Option<ReportToast> {
    if report.is_empty() {
        return None;
    }

    let groups = [
        group("new", &report.added),
        group("new, not downloaded", &report.added_not_fetched),
        group("updated", &report.updated),
        group("removed", &report.removed),
    ]
    .into_iter()
    .flatten()
    .collect();

    // `Success` only when the revision left nothing behind. Anything the scope
    // listed and did not fetch is still outstanding, and reporting that as a
    // success would be the completeness claim the sync scope forbids.
    let kind = if report.added_not_fetched.is_empty() {
        ToastKind::Success
    } else {
        ToastKind::Info
    };

    Some(ReportToast {
        kind,
        title: namespace.to_string(),
        // States the event, not the mechanism: whether it arrived by this pull,
        // the tick, or a resumed namespace is the status banner's business.
        body: "Updated to a newer revision.".to_owned(),
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn report() -> PullReport {
        PullReport {
            manifest_uri: quilt_uri::ManifestUri {
                bucket: "b".to_string(),
                namespace: ("acme", "demo").into(),
                hash: "h".to_string(),
                origin: None,
            },
            added: Vec::new(),
            added_not_fetched: Vec::new(),
            updated: Vec::new(),
            removed: Vec::new(),
            message: None,
        }
    }

    fn namespace() -> Namespace {
        ("acme", "rna-seq").into()
    }

    #[test]
    fn a_metadata_only_revision_says_nothing() {
        // The hashes advanced and no file moved. An entry standing until
        // dismissed that says nothing about files is pure noise.
        assert_eq!(report_toast(&namespace(), &report()), None);
    }

    /// A toast that opened with a package name and a file count never said why
    /// it was on screen. The lead sentence is that.
    #[test]
    fn the_toast_states_the_event_not_only_its_detail() {
        let r = PullReport {
            added: paths(&["qc/summary.csv"]),
            ..report()
        };
        let toast = report_toast(&namespace(), &r).unwrap();
        assert_eq!(toast.title, "acme/rna-seq");
        assert_eq!(toast.body, "Updated to a newer revision.");
    }

    /// Paths travel as data, not as indented text: whitespace stops meaning
    /// anything the moment a long path wraps.
    #[test]
    fn each_group_carries_its_paths_as_items() {
        let r = PullReport {
            added: paths(&["qc/summary.csv", "qc/flags.json"]),
            updated: paths(&["reads/day2.fastq"]),
            removed: paths(&["old/notes.md"]),
            ..report()
        };
        let toast = report_toast(&namespace(), &r).unwrap();
        let headings: Vec<&str> = toast.groups.iter().map(|g| g.heading.as_str()).collect();
        assert_eq!(
            headings,
            vec!["2 files new", "1 file updated", "1 file removed"]
        );
        assert_eq!(toast.groups[0].items, ["qc/summary.csv", "qc/flags.json"]);
        assert!(toast.groups.iter().all(|g| g.more == 0));
    }

    #[test]
    fn the_two_new_file_groups_read_differently() {
        // The same revision under the two sync scopes. If these read the same,
        // one of them is lying about whether the bytes are on disk.
        let fetched = PullReport {
            added: paths(&["qc/summary.csv"]),
            ..report()
        };
        let listed = PullReport {
            added_not_fetched: paths(&["qc/summary.csv"]),
            ..report()
        };
        assert_eq!(
            report_toast(&namespace(), &fetched).unwrap().groups[0].heading,
            "1 file new"
        );
        assert_eq!(
            report_toast(&namespace(), &listed).unwrap().groups[0].heading,
            "1 file new, not downloaded"
        );
    }

    #[test]
    fn anything_left_on_the_remote_is_not_a_success() {
        let all_here = PullReport {
            added: paths(&["a.csv"]),
            ..report()
        };
        let some_left = PullReport {
            added: paths(&["a.csv"]),
            added_not_fetched: paths(&["b.csv"]),
            ..report()
        };
        assert_eq!(
            report_toast(&namespace(), &all_here).unwrap().kind,
            ToastKind::Success
        );
        assert_eq!(
            report_toast(&namespace(), &some_left).unwrap().kind,
            ToastKind::Info
        );
    }

    #[test]
    fn a_long_group_names_some_and_counts_the_rest() {
        let r = PullReport {
            added_not_fetched: paths(&["a", "b", "c", "d", "e"]),
            ..report()
        };
        let group = &report_toast(&namespace(), &r).unwrap().groups[0];
        assert_eq!(group.heading, "5 files new, not downloaded");
        assert_eq!(group.items, ["a", "b", "c"]);
        assert_eq!(group.more, 2, "the remainder must be counted, not dropped");
    }
}
