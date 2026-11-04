//! The inbox: the bell's marks, the panel in each state, and the real one on
//! an appbar.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Appbar;
use crate::kit::Button;
use crate::kit::Cause;
use crate::kit::EntryKind;
use crate::kit::FileChange;
use crate::kit::FileGroup;
use crate::kit::FileReport;
use crate::kit::Inbox;
use crate::kit::InboxEntry;
use crate::kit::InboxPanel;
use crate::kit::NeedsYou;
use crate::kit::Revision;
use crate::kit::icons;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;

fn paths(prefix: &str, n: usize) -> Vec<String> {
    (1..=n)
        .map(|i| format!("{prefix}/well-{i:03}.csv"))
        .collect()
}

fn group(change: FileChange, prefix: &str, count: usize) -> FileGroup {
    // The record keeps the counts whole and at most 200 paths.
    FileGroup {
        change,
        count,
        paths: paths(prefix, count.min(200)),
    }
}

fn entry(id: u64, kind: EntryKind, package: &str, ago: f64, report: FileReport) -> InboxEntry {
    InboxEntry {
        id,
        kind,
        package: package.to_string(),
        revision: None,
        report,
        outcome: None,
        at: js_sys::Date::now() - ago,
        unread: false,
        href: "#".to_string(),
    }
}

/// One of each kind, oldest last, the newest two unread.
pub fn every_kind() -> Vec<InboxEntry> {
    vec![
        InboxEntry {
            unread: true,
            ..entry(
                1,
                EntryKind::Got,
                "user/plate-07",
                4.0 * MINUTE,
                FileReport::Files(vec![
                    group(FileChange::New, "plates/07", 2),
                    group(FileChange::Updated, "qc", 1),
                ]),
            )
        },
        InboxEntry {
            unread: true,
            ..entry(
                2,
                EntryKind::Published,
                "lab/rna-seq",
                25.0 * MINUTE,
                FileReport::Files(vec![group(FileChange::Updated, "counts", 5)]),
            )
        },
        InboxEntry {
            revision: Some(Revision {
                from: Some("1a2b3c4".to_string()),
                to: "5d6e7f8".to_string(),
            }),
            ..entry(
                3,
                EntryKind::Outside { published: false },
                "lab/imaging",
                2.0 * HOUR,
                FileReport::Differs(vec![group(FileChange::Updated, "tiles", 2)]),
            )
        },
        InboxEntry {
            outcome: Some("Freed 1.2 GB of downloaded files.".to_string()),
            href: "#/".to_string(),
            ..entry(
                4,
                EntryKind::Uninstalled,
                "old/screen-2024",
                5.0 * HOUR,
                FileReport::None,
            )
        },
        entry(
            5,
            EntryKind::Created,
            "user/new-run",
            26.0 * HOUR,
            FileReport::Files(vec![group(FileChange::LeftOut, ".cache", 4)]),
        ),
        InboxEntry {
            outcome: Some("Freed 340 MB.".to_string()),
            ..entry(
                6,
                EntryKind::RemovedRevisions { count: 3 },
                "lab/rna-seq",
                3.0 * 24.0 * HOUR,
                FileReport::None,
            )
        },
    ]
}

/// A big pull: the record kept 200 of 283 paths, and the counts stay whole.
fn big_pull() -> Vec<InboxEntry> {
    vec![InboxEntry {
        unread: true,
        ..entry(
            1,
            EntryKind::Got,
            "a-team-with-a-long-name/an-analysis-package-whose-name-goes-on",
            MINUTE,
            FileReport::Files(vec![
                group(
                    FileChange::New,
                    "raw/2026-11-03/instrument-b/plate-batch-with-a-very-long-folder-name",
                    283,
                ),
                group(FileChange::NewNotDownloaded, "raw/big", 12),
                group(FileChange::Removed, "scratch", 1),
            ]),
        )
    }]
}

/// The same pulls, one entry each, never merged.
fn three_pulls() -> Vec<InboxEntry> {
    (1..=3)
        .map(|i| {
            entry(
                i,
                EntryKind::Got,
                "user/plate-07",
                f64::from(u32::try_from(i).unwrap_or(0)) * 10.0 * MINUTE,
                FileReport::Files(vec![group(FileChange::Updated, "qc", 1)]),
            )
        })
        .collect()
}

pub fn needs_you_in_queue(count: usize) -> NeedsYou {
    NeedsYou {
        in_queue: count,
        queue_href: "#/".to_string(),
        elsewhere: Vec::new(),
    }
}

fn needs_you_everywhere() -> NeedsYou {
    let now = js_sys::Date::now();
    NeedsYou {
        in_queue: 2,
        queue_href: "#/".to_string(),
        elsewhere: vec![
            Cause::watcher_stopped("#/settings"),
            Cause::publish_failing("user/plate-07", now - 2.5 * HOUR, now, "#"),
        ],
    }
}

/// Settings beside the bell, as the bar draws its controls: the bell first.
fn bar(bell: AnyView) -> AnyView {
    view! {
        <div class="g-stack">
            <Appbar actions=Some(
                view! {
                    {bell}
                    <Button leading_visual=icons::sync() on_click=|_| ()>
                        "Refresh"
                    </Button>
                    <Button leading_visual=icons::gear() on_click=|_| ()>
                        "Settings"
                    </Button>
                }
                    .into_any(),
            ) />
        </div>
    }
    .into_any()
}

fn bell(entries: Vec<InboxEntry>, needs_you: NeedsYou) -> AnyView {
    let open = RwSignal::new(false);
    bar(view! { <Inbox entries=entries needs_you=needs_you open=open /> }.into_any())
}

fn panel(entries: Vec<InboxEntry>, needs_you: NeedsYou) -> AnyView {
    view! {
        <div class="g-popover-inline">
            <InboxPanel entries=entries needs_you=needs_you />
        </div>
    }
    .into_any()
}

#[component]
pub fn InboxStories() -> impl IntoView {
    view! {
        <Bells />
        <Panels />
        <Live />
    }
}

#[component]
fn Bells() -> impl IntoView {
    view! {
        <Story
            title="Inbox · the bell"
            note="Leads the appbar's controls. Two marks or none: a quiet grey dot while \
                  any entry is unread, and an amber count of what needs you, which wins \
                  when both hold and drops only when a cause clears — opening the inbox \
                  does not clear it. The accessible name says the same in words: \
                  Inbox, Inbox, unread news, or Inbox, 2 need you. Each bell opens."
        >
            <Cell full=true label="nothing new — Inbox">
                {bell(Vec::new(), NeedsYou::default())}
            </Cell>
            <Cell full=true label="unread news — Inbox, unread news">
                {bell(every_kind(), NeedsYou::default())}
            </Cell>
            <Cell full=true label="needs you — Inbox, 1 needs you">
                {bell(Vec::new(), needs_you_in_queue(1))}
            </Cell>
            <Cell full=true label="both hold: the count wins — Inbox, 4 need you">
                {bell(every_kind(), needs_you_everywhere())}
            </Cell>
            <Cell full=true label="two digits — Inbox, 12 need you">
                {bell(Vec::new(), needs_you_in_queue(12))}
            </Cell>
        </Story>
    }
}

#[component]
fn Panels() -> impl IntoView {
    view! {
        <Story
            title="Inbox · the panel"
            note="The record, newest first, one entry per event and never merged. Each entry \
                  is one link to its package's page — an uninstalled one's to the main page \
                  — with no dismiss: opening the panel is what reads them. The lead says what \
                  happened to which package, then the file report as headed lists: the \
                  heading counts the whole group, the list names the first three and states \
                  the rest. Unread entries are tinted and edged. Drawn inline here, at the \
                  panel's width; the last story opens the real popover."
        >
            <Cell label="nothing new">{panel(Vec::new(), NeedsYou::default())}</Cell>
            <Cell label="nothing new, but 2 need you">
                {panel(Vec::new(), needs_you_in_queue(2))}
            </Cell>
            <Cell wide=true label="every kind, the newest two unread">
                {panel(every_kind(), NeedsYou::default())}
            </Cell>
            <Cell wide=true label="needs you: the queue's count, then the causes it does not show">
                {panel(every_kind(), needs_you_everywhere())}
            </Cell>
            <Cell label="three pulls are three entries">
                {panel(three_pulls(), NeedsYou::default())}
            </Cell>
            <Cell label="a long name and 283 files, 200 of them kept">
                {panel(big_pull(), NeedsYou::default())}
            </Cell>
        </Story>
    }
}

/// The real popover, with a stand-in for the backend reading the entries once
/// the panel has stayed open.
#[component]
fn Live() -> impl IntoView {
    let entries = RwSignal::new(every_kind());
    let needs_you = RwSignal::new(needs_you_in_queue(0));
    let open = RwSignal::new(false);
    // The backend's dwell: read once the panel has stayed open for 2 s.
    Effect::new(move |_| {
        if open.get() {
            set_timeout(
                move || {
                    if open.get_untracked() {
                        entries.update(|all| all.iter_mut().for_each(|e| e.unread = false));
                    }
                },
                std::time::Duration::from_secs(2),
            );
        }
    });
    let arrive = move |_| {
        entries.update(|all| {
            let id = all.iter().map(|e| e.id).max().unwrap_or(0) + 1;
            all.push(InboxEntry {
                unread: true,
                ..entry(
                    id,
                    EntryKind::Got,
                    "user/plate-07",
                    0.0,
                    FileReport::Files(vec![group(FileChange::Updated, "qc", 1)]),
                )
            });
        });
    };
    let toggle_needs = move |_| {
        needs_you.update(|n| n.in_queue = if n.in_queue == 0 { 2 } else { 0 });
    };

    view! {
        <Story
            title="Inbox · the real one"
            note="The kit's popover, anchored to the bell: Escape and a click outside close \
                  it. Open it and wait two seconds: a stand-in for the backend reads the \
                  entries, the dot goes, and the entries that were unread keep their tint \
                  until the panel closes. Close it at once and nothing is read. A new pull \
                  arrives unread and lights the dot again; the count is the needs-you \
                  set's alone."
        >
            <Cell full=true label="live">
                <div class="g-stack">
                    {bar(view! { <Inbox entries=entries needs_you=needs_you open=open /> }.into_any())}
                    <div class="g-inline">
                        <Button on_click=arrive>"A pull arrives"</Button>
                        <Button on_click=toggle_needs>"Needs you: 0 or 2"</Button>
                    </div>
                </div>
            </Cell>
        </Story>
    }
}
