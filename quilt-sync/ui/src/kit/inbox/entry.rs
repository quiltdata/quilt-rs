//! One inbox entry: what happened to which package and when, then its file
//! report as headed lists. The whole entry is one link.
//!
//! A heading counts its whole group and the list names the first few, then
//! states the rest, as the notification stack's entries do. The report is
//! text, not links.

use leptos::prelude::*;

use super::EntryKind;
use super::FileGroup;
use super::FileReport;
use super::InboxEntry;
use crate::kit::RelativeTime;

stylance::import_crate_style!(style, "src/kit/inbox/entry.module.css");

/// How many paths a group names before it counts the rest.
const NAMED: usize = 3;

fn files(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

/// What happened, after the package's name.
pub(super) fn what(entry: &InboxEntry) -> String {
    let total = match &entry.report {
        FileReport::Files(groups) => groups.iter().map(|g| g.count).sum(),
        FileReport::None | FileReport::Differs(_) => 0,
    };
    match &entry.kind {
        EntryKind::Got if total > 0 => format!("got {}", files(total)),
        EntryKind::Got => "got a newer revision".to_string(),
        EntryKind::Published if total > 0 => format!("published {}", files(total)),
        EntryKind::Published => "published a revision".to_string(),
        EntryKind::Uninstalled => "uninstalled".to_string(),
        EntryKind::Created => "created".to_string(),
        EntryKind::RemovedRevisions { count: 1 } => "removed 1 old revision".to_string(),
        EntryKind::RemovedRevisions { count } => format!("removed {count} old revisions"),
        EntryKind::Outside { published: true } => {
            "published on this computer, outside QuiltSync".to_string()
        }
        EntryKind::Outside { published: false } => {
            "updated on this computer, outside QuiltSync".to_string()
        }
    }
}

fn group_view(group: FileGroup) -> impl IntoView {
    let FileGroup {
        change,
        count,
        paths,
    } = group;
    let shown: Vec<String> = paths.into_iter().take(NAMED).collect();
    let more = count.saturating_sub(shown.len());
    let items = shown
        .into_iter()
        .map(|path| view! { <li>{path}</li> })
        .collect_view();
    view! {
        <div class=style::group>
            <span class=style::heading>{format!("{} {}", files(count), change.words())}</span>
            <ul class=style::items>
                {items}
                {(more > 0).then(|| view! { <li class=style::more>{format!("and {more} more")}</li> })}
            </ul>
        </div>
    }
}

#[component]
pub(super) fn EntryView(
    entry: InboxEntry,
    #[prop(into)] highlighted: Signal<bool>,
) -> impl IntoView {
    let what = what(&entry);
    let InboxEntry {
        kind,
        package,
        revision,
        report,
        outcome,
        at,
        href,
        ..
    } = entry;
    // Only a change made outside QuiltSync names its revisions: there the
    // move is the news, and nothing else says which way it went.
    let revision = matches!(kind, EntryKind::Outside { .. })
        .then_some(revision)
        .flatten()
        .map(|r| match r.from {
            Some(from) => format!("From {from} to {}", r.to),
            None => format!("To {}", r.to),
        });
    let (differs, groups) = match report {
        FileReport::None => (false, Vec::new()),
        FileReport::Files(groups) => (false, groups),
        FileReport::Differs(groups) => (true, groups),
    };
    let groups = groups.into_iter().map(group_view).collect_view();
    let class = move || {
        if highlighted.get() {
            format!("{} {}", style::entry, style::unread)
        } else {
            style::entry.to_string()
        }
    };

    view! {
        <a class=class href=href>
            <span class=style::line>
                <span class=style::what>
                    {move || highlighted.get().then(|| view! { <span data-sr-only>"Unread: "</span> })}
                    <span class=style::package>{package}</span>
                    ": "
                    {what}
                </span>
                <span class=style::time>
                    <RelativeTime at=at />
                </span>
            </span>
            {revision.map(|r| view! { <span class=style::note>{r}</span> })}
            {differs.then(|| view! { <span class=style::note>"Differs between revisions"</span> })}
            {groups}
            {outcome.map(|o| view! { <span class=style::outcome>{o}</span> })}
        </a>
    }
}

#[cfg(test)]
mod tests {
    use super::super::FileChange;
    use super::super::Revision;
    use super::*;

    fn entry(kind: EntryKind, report: FileReport) -> InboxEntry {
        InboxEntry {
            id: 1,
            kind,
            package: "user/plate-07".to_string(),
            revision: Some(Revision {
                from: Some("1a2b3c4".to_string()),
                to: "5d6e7f8".to_string(),
            }),
            report,
            outcome: None,
            at: 0.0,
            unread: false,
            href: "/x".to_string(),
        }
    }

    fn group(change: FileChange, count: usize) -> FileGroup {
        FileGroup {
            change,
            count,
            paths: (0..count.min(200)).map(|i| format!("f{i}.csv")).collect(),
        }
    }

    #[test]
    fn a_pull_says_how_many_files_it_got() {
        let report = FileReport::Files(vec![
            group(FileChange::New, 2),
            group(FileChange::Updated, 1),
        ]);
        assert_eq!(what(&entry(EntryKind::Got, report)), "got 3 files");
        assert_eq!(
            what(&entry(EntryKind::Got, FileReport::None)),
            "got a newer revision"
        );
    }

    #[test]
    fn a_change_outside_says_which_way_when_it_can() {
        assert_eq!(
            what(&entry(
                EntryKind::Outside { published: false },
                FileReport::None
            )),
            "updated on this computer, outside QuiltSync"
        );
        assert_eq!(
            what(&entry(
                EntryKind::Outside { published: true },
                FileReport::None
            )),
            "published on this computer, outside QuiltSync"
        );
    }

    /// The files that differ are not what the outside tool wrote, so they do
    /// not count as files it got.
    #[test]
    fn a_difference_is_not_counted_as_files_moved() {
        let report = FileReport::Differs(vec![group(FileChange::Updated, 4)]);
        assert_eq!(
            what(&entry(EntryKind::Outside { published: false }, report)),
            "updated on this computer, outside QuiltSync"
        );
    }
}
