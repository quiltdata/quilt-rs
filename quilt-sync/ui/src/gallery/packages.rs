//! `PackageRow` stories and the packages-view scene.
//!
//! The scene is the point. A row in isolation says nothing about whether forty
//! `Latest` labels read as a list or as a wall of green, and that is the open
//! question the design has about this view.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::EmptyState;
use crate::kit::GroupHeader;
use crate::kit::ListToolbar;
use crate::kit::PackageRow;
use crate::kit::SearchInput;
use crate::kit::Select;
use crate::kit::StateTone;
use crate::kit::ViewToggle;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

#[component]
pub fn PackageRowStories() -> impl IntoView {
    view! { <StatesStory /><EdgesStory /> }
}

/// The ten states again, but on a row — which is where they are actually read, and
/// where the ragged left edge of the state column becomes visible.
#[component]
fn StatesStory() -> impl IntoView {
    let states: Vec<(&str, StateTone, f64)> = vec![
        ("Latest", StateTone::Success, 2.0 * HOUR),
        ("Not the latest", StateTone::Attention, 3.0 * DAY),
        ("Newer revision available", StateTone::Attention, 20.0 * MINUTE),
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

#[component]
fn EdgesStory() -> impl IntoView {
    view! {
        <Story
            title="PackageRow — edges"
            note="Namespaces truncate from the RIGHT, the opposite of FileRow's paths: a \
                  namespace is distinguished by its start, and there is no filename at the \
                  end worth saving. The time column is a fixed 80px — the last cell is the \
                  worst-case phrase set, and if any of those ellipsise the column wants 96px \
                  or the phrases want shortening."
        >
            <Cell full=true label="long namespace truncates right, state keeps its place">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Newer revision available"
                    tone=StateTone::Attention
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
                    state="Newer revision available"
                    tone=StateTone::Attention
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
            <Cell full=true label="the 80px time column against its worst cases">
                {[
                    ("just now", 10.0 * 1000.0),
                    ("18 min ago", 18.0 * MINUTE),
                    ("23 hours ago", 23.0 * HOUR),
                    ("yesterday", 30.0 * HOUR),
                    ("3 weeks ago", 21.0 * DAY),
                    ("2 months ago", 70.0 * DAY),
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
            </Cell>
        </Story>
    }
}

/// Forty-three packages, three of which need something — the real ratio, because
/// the ratio is what the open question is about.
fn fixtures() -> Vec<(&'static str, &'static str, &'static str, StateTone, f64)> {
    let latest = ("Latest", StateTone::Success);
    let mut rows: Vec<(&str, &str, &str, StateTone, f64)> = vec![
        ("s3://my-bucket", "user/package-a", "2 files changed", StateTone::Neutral, 8.0 * MINUTE),
        ("s3://my-bucket", "user/package-b", "Newer revision available", StateTone::Attention, 20.0 * MINUTE),
        ("s3://my-bucket", "user/qc-plates-2026-08", latest.0, latest.1, 2.0 * HOUR),
        ("s3://my-bucket", "user/scratch", latest.0, latest.1, 5.0 * HOUR),
        ("s3://my-bucket", "user/reference-genomes", latest.0, latest.1, 26.0 * HOUR),
        ("s3://team-bucket", "team/rnaseq-batch-2026-07-31-reprocessed-v2", "No access", StateTone::Danger, 3.0 * DAY),
        ("s3://team-bucket", "team/imaging-cohort-b", "No access", StateTone::Danger, 9.0 * DAY),
        ("s3://org-archive", "org/dataset-c", latest.0, latest.1, 21.0 * DAY),
        ("s3://org-archive", "org/dataset-d", latest.0, latest.1, 70.0 * DAY),
    ];
    // Pad the healthy bucket, because five green rows and forty green rows are
    // different design questions and only one of them is ours.
    for i in 0..8 {
        rows.push((
            "s3://my-bucket",
            match i {
                0 => "user/assay-controls",
                1 => "user/instrument-logs",
                2 => "user/pilot-2026-06",
                3 => "user/pilot-2026-07",
                4 => "user/annotations",
                5 => "user/manifests",
                6 => "user/thumbnails",
                _ => "user/exports",
            },
            latest.0,
            latest.1,
            f64::from(i + 2) * DAY,
        ));
    }
    rows
}

/// Two spellings of the same header, because `GroupHeader`'s `annotation` is
/// `#[prop(optional, into)]` and Leptos makes that setter take `impl Into<String>`
/// — so an `Option<String>` cannot be forwarded to it. Harmless here, where the
/// condition is a fixture, but the real page reads the cause off a DTO as an
/// `Option` and will hit this. Noted rather than fixed: changing the prop would
/// cost every caller that passes a literal.
fn header(bucket: &'static str, count: usize) -> AnyView {
    // Only the bucket axis carries a cause: a prefix spans buckets, so no cause is
    // a property of the group.
    if bucket == "s3://team-bucket" {
        view! { <GroupHeader title=bucket count=count annotation="no access as analyst" /> }
            .into_any()
    } else {
        view! { <GroupHeader title=bucket count=count /> }.into_any()
    }
}

fn row(entry: (&'static str, &'static str, &'static str, StateTone, f64)) -> AnyView {
    let (_, namespace, state, tone, elapsed) = entry;
    view! {
        <PackageRow
            namespace=namespace
            href="#scene-packages"
            changed_at=ago(elapsed)
            state=state
            tone=tone
        />
    }
    .into_any()
}

#[component]
pub fn PackagesScene() -> impl IntoView {
    let view_mode = RwSignal::new("Packages".to_string());
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Recently changed".to_string());

    view! {
        <Scene
            title="Scene · packages"
            note="Seventeen rows at the real ratio: three need something, the rest are \
                  Latest. THE OPEN QUESTION IS HERE — does a column of Latest read as a \
                  calm list, or as a wall of green that spends the signal the tone was \
                  bought for? Switch the theme; light and dark disagree about this more \
                  than about anything else in the kit. \
                  \
                  The toolbar differs from the files view: this one adds Sort and Create \
                  package, and Group offers Bucket / Prefix / None rather than \
                  None / Package. ListToolbar holds nothing itself, which is what lets \
                  the two views compose different controls."
        >
            <ListToolbar>
                <ViewToggle
                    label="List view"
                    name="packages-view"
                    options=vec!["Packages".to_string(), "Recent files".to_string()]
                    selected=view_mode
                />
                <SearchInput value=query label="Search packages" placeholder="Search…" />
                <Select
                    label="Group"
                    options=vec![
                        "Bucket".to_string(),
                        "Prefix".to_string(),
                        "None".to_string(),
                    ]
                    selected=group
                    visible_label=true
                />
                <Select
                    label="Sort"
                    options=vec!["Recently changed".to_string(), "Name".to_string()]
                    selected=sort
                    visible_label=true
                />
                <Button variant=ButtonVariant::Primary on_click=|_| ()>
                    "Create package"
                </Button>
            </ListToolbar>
            {move || {
                let rows = fixtures();
                if group.get() == "Bucket" {
                    let mut order: Vec<&'static str> = Vec::new();
                    for (bucket, ..) in &rows {
                        if !order.contains(bucket) {
                            order.push(*bucket);
                        }
                    }
                    order
                        .into_iter()
                        .map(|bucket| {
                            let group_rows: Vec<_> = rows
                                .iter()
                                .copied()
                                .filter(|(b, ..)| *b == bucket)
                                .collect();
                            view! {
                                {header(bucket, group_rows.len())}
                                {group_rows.into_iter().map(row).collect_view()}
                            }
                                .into_any()
                        })
                        .collect_view()
                        .into_any()
                } else {
                    rows.into_iter().map(row).collect_view().into_any()
                }
            }}
        </Scene>
        <Scene
            title="Scene · packages, no results"
            note="One EmptyState component for both empties. No results carries no action — \
                  the user already knows how to change their own search, so a button here \
                  would be filler."
        >
            <EmptyState
                title="No packages match “plate-99”"
                body="Search covers the names of packages installed on this machine."
            />
        </Scene>
    }
}
