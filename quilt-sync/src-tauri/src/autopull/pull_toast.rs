//! The wording of a pull's [report](quilt_rs::flow::PullReport), as a toast.
//!
//! Composed here rather than in the client because the groups are the report's
//! own: the stack renders what it is given, so the copy lives beside the tick
//! that posts it and nothing in the UI needs to know a revision exists.

use std::fmt::Write as _;

use quilt_rs::flow::PullReport;
use quilt_uri::Namespace;
use std::path::PathBuf;

use crate::toast::ToastKind;

/// How many paths a group names before it counts the rest.
///
/// A toast has one line's worth of room and the point is that the user can see
/// *which* files, not merely how many — so it names some and is honest about
/// the remainder rather than truncating silently.
const NAMED: usize = 3;

/// A toast's three parts, ready for [`ToastCenter::post`](crate::toast::ToastCenter::post).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportToast {
    pub kind: ToastKind,
    pub title: String,
    pub body: String,
}

/// One group: a count, then up to [`NAMED`] paths, then the remainder.
fn group(heading: &str, paths: &[PathBuf]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let plural = if paths.len() == 1 { "" } else { "s" };
    let mut lines = format!("{} file{plural} {heading}", paths.len());
    for path in paths.iter().take(NAMED) {
        let _ = write!(lines, "\n  {}", path.display());
    }
    if let Some(rest) = paths.len().checked_sub(NAMED).filter(|rest| *rest > 0) {
        let _ = write!(lines, "\n  and {rest} more");
    }
    Some(lines)
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

    let body = [
        group("new", &report.added),
        group("new, not downloaded", &report.added_not_fetched),
        group("updated", &report.updated),
        group("removed", &report.removed),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");

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
        body,
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

    #[test]
    fn each_group_names_its_files_under_a_count() {
        let r = PullReport {
            added: paths(&["qc/summary.csv", "qc/flags.json"]),
            updated: paths(&["reads/day2.fastq"]),
            removed: paths(&["old/notes.md"]),
            ..report()
        };
        let toast = report_toast(&namespace(), &r).expect("something moved");
        assert_eq!(toast.title, "acme/rna-seq");
        assert_eq!(
            toast.body,
            "2 files new\n  qc/summary.csv\n  qc/flags.json\n\
             1 file updated\n  reads/day2.fastq\n\
             1 file removed\n  old/notes.md"
        );
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
        let fetched = report_toast(&namespace(), &fetched).unwrap();
        let listed = report_toast(&namespace(), &listed).unwrap();
        assert_eq!(fetched.body, "1 file new\n  qc/summary.csv");
        assert_eq!(listed.body, "1 file new, not downloaded\n  qc/summary.csv");
        assert_ne!(fetched.body, listed.body);
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
        let toast = report_toast(&namespace(), &r).unwrap();
        assert_eq!(
            toast.body,
            "5 files new, not downloaded\n  a\n  b\n  c\n  and 2 more"
        );
    }
}
