//! `PackageRow` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::PackageRow;
use crate::kit::RowDownloads;
use crate::kit::StateTone;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

#[component]
pub fn PackageRowStories() -> impl IntoView {
    view! { <StatesStory /><DownloadsStory /><TooltipStory /><EdgesStory /> }
}

/// The nine states again, but on a row — which is where they are actually read, and
/// where the ragged left edge of the state column becomes visible. The queue's own
/// wordings are clauses and never render in a chip, so every label here is the
/// list's.
#[component]
fn StatesStory() -> impl IntoView {
    let states: Vec<(&str, StateTone, f64)> = vec![
        ("Latest", StateTone::Success, 2.0 * HOUR),
        ("Not the latest", StateTone::Attention, 3.0 * DAY),
        ("2 files changed", StateTone::Neutral, 8.0 * MINUTE),
        ("conflicts in 2 files", StateTone::Danger, 40.0 * MINUTE),
        ("Changed in both places", StateTone::Danger, 5.0 * HOUR),
        ("No access", StateTone::Danger, 30.0 * DAY),
        ("No S3 bucket yet", StateTone::Attention, 26.0 * HOUR),
        ("Not published yet", StateTone::Attention, 45.0 * MINUTE),
        ("Revision not published", StateTone::Attention, 9.0 * DAY),
    ];

    view! {
        <Story
            title="PackageRow"
            note="The whole row is one anchor — hover anywhere, and middle-click or \
                  Ctrl+click behave as links because it is a real href, not a click \
                  handler. No underline on hover: the background tint is the affordance \
                  and two signals for one target is noise. Tab through: one stop per row, \
                  and the ring is around the row rather than around anything inside it. \
                  \
                  The state column is right-aligned and ragged on the left, on purpose — \
                  see the last cell of the scene below for why."
        >
            {states
                .into_iter()
                .map(|(state, tone, elapsed)| {
                    view! {
                        <Cell full=true label=state>
                            <PackageRow
                                namespace="user/package-a"
                                href="#packagerow"
                                changed_at=ago(elapsed)
                                state=state
                                tone=tone
                            />
                        </Cell>
                    }
                })
                .collect_view()}
        </Story>
    }
}

/// The download markers. A v2 row only: the whole-package scope exists only there.
#[component]
fn DownloadsStory() -> impl IntoView {
    let cells: Vec<(&str, RowDownloads, &str, StateTone)> = vec![
        (
            "whole package, every file downloaded: nothing extra",
            RowDownloads::EntirePackage { to_download: 0 },
            "Latest",
            StateTone::Success,
        ),
        (
            "whole package, one file outstanding",
            RowDownloads::EntirePackage { to_download: 1 },
            "Latest",
            StateTone::Success,
        ),
        (
            "whole package, files outstanding beside local changes",
            RowDownloads::EntirePackage { to_download: 3 },
            "2 files changed",
            StateTone::Neutral,
        ),
        (
            "whole package, files outstanding with a newer remote revision",
            RowDownloads::EntirePackage { to_download: 3 },
            "Not the latest",
            StateTone::Attention,
        ),
        (
            "picked files",
            RowDownloads::IndividualFiles {
                downloaded: 12,
                total: 340,
            },
            "Latest",
            StateTone::Success,
        ),
        (
            "picked files, with a newer remote revision",
            RowDownloads::IndividualFiles {
                downloaded: 1,
                total: 58,
            },
            "Not the latest",
            StateTone::Attention,
        ),
    ];

    view! {
        <Story
            title="PackageRow — downloads"
            note="What is on disk, as a badge between the time and the state label. A whole \
                  package with every file downloaded shows nothing extra; one with files \
                  outstanding shows how many, filled in the accent because the reader can \
                  act on it; a package kept to picked files shows how many of the \
                  package's files are on disk, as a grey outline. The state labels are \
                  unchanged."
        >
            {cells
                .into_iter()
                .map(|(label, downloads, state, tone)| {
                    view! {
                        <Cell full=true label=label>
                            <PackageRow
                                namespace="user/package-a"
                                href="#packagerow"
                                changed_at=ago(3.0 * HOUR)
                                state=state
                                tone=tone
                                downloads=downloads
                            />
                        </Cell>
                    }
                })
                .collect_view()}
            <Cell wide=true label="narrow — the namespace gives way, the marker does not">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Changed in both places"
                    tone=StateTone::Danger
                    downloads=RowDownloads::EntirePackage { to_download: 128 }
                />
            </Cell>
        </Story>
    }
}

/// Label, namespace, host, scope, size.
type TooltipCell = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<RowDownloads>,
    Option<&'static str>,
);

/// What the row does not draw, on hover or focus. Nothing here is open: each
/// cell is a row to rest on or Tab to, as `Tooltip`'s own stories are.
#[component]
fn TooltipStory() -> impl IntoView {
    const HOST: Option<&str> = Some("nightly.quilttest.com");
    let picked = Some(RowDownloads::IndividualFiles {
        downloaded: 12,
        total: 340,
    });
    // The package page's Keeping words, as `util::counted` puts them.
    let picked_size = Some("12 of 340 files \u{b7} 1.9\u{a0}GB of 3.4\u{a0}GB downloaded");
    let entire = Some(RowDownloads::EntirePackage { to_download: 0 });
    let cells: Vec<TooltipCell> = vec![
        (
            "picked files, some downloaded",
            "team/package-a",
            HOST,
            picked,
            picked_size,
        ),
        (
            "entire package, all downloaded",
            "team/package-b",
            HOST,
            entire,
            Some("All files are downloaded \u{b7} 86.3\u{a0}GB"),
        ),
        (
            "entire package, nothing downloaded yet",
            "team/package-c",
            HOST,
            Some(RowDownloads::EntirePackage { to_download: 58 }),
            Some("No files are downloaded \u{b7} 86.3\u{a0}GB"),
        ),
        (
            "not checked yet — the scope and size lines wait for the check",
            "team/package-d",
            HOST,
            None,
            None,
        ),
        (
            "no host — its line is left out",
            "team/package-e",
            None,
            entire,
            Some("All files are downloaded \u{b7} 44.0\u{a0}kB"),
        ),
        (
            "a very long name — whole in the title, wrapped at the wider measure",
            "team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix-that-no-column-holds",
            HOST,
            picked,
            picked_size,
        ),
    ];

    view! {
        <Story
            title="PackageRow — tooltip"
            note="Rest the pointer anywhere on a row for half a second, or Tab to it: the \
                  name in full, then in grey, each after its glyph, the host, what the \
                  package keeps and how much of it is here, in the Keeping section's \
                  words, with no full stops. A line whose fact is missing is left out. It \
                  hangs below the row from its left edge, and above when there is no room \
                  below; the pointer passes through it to the rows it covers. Switch the \
                  theme to check it in dark: the grey is a solid colour, not the title \
                  dimmed."
        >
            {cells
                .into_iter()
                .map(|(label, namespace, host, downloads, size)| {
                    view! {
                        <Cell full=true label=label>
                            <PackageRow
                                namespace=namespace
                                href="#packagerow"
                                changed_at=ago(3.0 * HOUR)
                                state="Latest"
                                tone=StateTone::Success
                                downloads=downloads
                                host=host.map(str::to_string)
                                size=size.map(str::to_string)
                            />
                        </Cell>
                    }
                })
                .collect_view()}
        </Story>
    }
}

#[component]
fn EdgesStory() -> impl IntoView {
    view! {
        <Story
            title="PackageRow — edges"
            note="Namespaces truncate from the right, the opposite of FileRow's paths: a \
                  namespace is distinguished by its start. The time column is a 104px floor, \
                  sized to the widest phrase RelativeTime can produce in the widest system \
                  face, and never a cap. The last cell is the worst-case phrase set, and \
                  every one of them has to sit inside the column. `Changed in both places`, \
                  at 22 characters, is the widest label a list row ever carries."
        >
            <Cell full=true label="long namespace truncates right, state keeps its place">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Changed in both places"
                    tone=StateTone::Danger
                />
            </Cell>
            <Cell full=true label="no recorded time — a word, never a blank cell">
                <PackageRow
                    namespace="user/package-a"
                    href="#packagerow"
                    state="Not published yet"
                    tone=StateTone::Attention
                />
            </Cell>
            <Cell wide=true label="narrow — two columns, roughly a narrow window">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Changed in both places"
                    tone=StateTone::Danger
                />
            </Cell>
            <Cell wide=true label="narrow · short state — the namespace gets the slack back">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2"
                    href="#packagerow"
                    changed_at=ago(2.0 * HOUR)
                    state="Latest"
                    tone=StateTone::Success
                />
            </Cell>
            <Cell full=true label="provisional — the guess dims and the label dashes; the timestamp does not, because it is not part of the guess">
                <div class="g-rows">
                    <PackageRow
                        namespace="user/package-a"
                        href="#packagerow"
                        changed_at=ago(2.0 * HOUR)
                        state="Latest"
                        tone=StateTone::Success
                        provisional=true
                    />
                    <PackageRow
                        namespace="user/package-b"
                        href="#packagerow"
                        changed_at=ago(20.0 * MINUTE)
                        state="Not the latest"
                        tone=StateTone::Attention
                        provisional=true
                    />
                    <PackageRow
                        namespace="user/package-c"
                        href="#packagerow"
                        changed_at=ago(5.0 * HOUR)
                        state="Latest"
                        tone=StateTone::Success
                    />
                </div>
            </Cell>
            <Cell full=true label="the time column against its widest phrases">
                <div class="g-rows">
                    {[
                        ("just now", 10.0 * 1000.0),
                        ("18 min ago", 18.0 * MINUTE),
                        ("23 hours ago", 23.0 * HOUR),
                        ("yesterday", 30.0 * HOUR),
                        ("3 weeks ago", 21.0 * DAY),
                        ("11 months ago", 340.0 * DAY),
                        ("3 years ago", 1100.0 * DAY),
                    ]
                        .into_iter()
                        .map(|(phrase, elapsed)| {
                            view! {
                                <PackageRow
                                    namespace=format!("expected “{phrase}”")
                                    href="#packagerow"
                                    changed_at=ago(elapsed)
                                    state="Latest"
                                    tone=StateTone::Success
                                />
                            }
                        })
                        .collect_view()}
                </div>
            </Cell>
        </Story>
    }
}
