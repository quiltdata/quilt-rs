//! `PackageRow` stories and the packages-view scene.
//!
//! The scene is the point. A row in isolation says nothing about whether forty
//! `Latest` labels read as a list or as a wall of green, and that is the open
//! question the design has about this view.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::GroupHeading;
use crate::kit::ListToolbar;
use crate::kit::Naming;
use crate::kit::PackageRow;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::kit::StateTone;

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

/// The nine states again, but on a row — which is where they are actually read, and
/// where the ragged left edge of the state column becomes visible.
///
/// Nine, not ten: `behind` renders as `Not the latest` here and as
/// `Newer revision available` on a queue row, so only the first can appear in this story.
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

#[component]
fn EdgesStory() -> impl IntoView {
    view! {
        <Story
            title="PackageRow — edges"
            note="Namespaces truncate from the RIGHT, the opposite of FileRow's paths: a \
                  namespace is distinguished by its start, and there is no filename at the \
                  end worth saving. The time column is a fixed 96px — widened from 80 once \
                  real timestamps landed on the page and the worst phrases could be read \
                  rather than guessed at. The last cell is the worst-case phrase set; if any \
                  of those still ellipsise, the phrases want shortening rather than the \
                  column want widening again. \
                  \
                  The state phrase in the truncation cells is `Changed in both places`, 22 \
                  characters, which is the widest label a LIST row can carry. It is not the \
                  widest label in the kit — `Newer revision available` is 24 — but that one \
                  only ever renders on a queue row, so sizing this column against it would \
                  be testing a case that cannot occur."
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
            <Cell full=true label="provisional — the light phase's guess, dashed until confirmed">
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
            <Cell full=true label="the 80px time column against its worst cases">
                <div class="g-rows">
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
                </div>
            </Cell>
        </Story>
    }
}

/// Forty-three packages, three of which need something — the real ratio, because
/// the ratio is what the open question is about.
fn fixtures() -> Vec<(&'static str, &'static str, &'static str, StateTone, f64)> {
    let latest = ("Latest", StateTone::Success);
    let mut rows: Vec<(&str, &str, &str, StateTone, f64)> = vec![
        (
            "s3://my-bucket",
            "user/package-a",
            "2 files changed",
            StateTone::Neutral,
            8.0 * MINUTE,
        ),
        (
            "s3://my-bucket",
            "user/package-b",
            "Not the latest",
            StateTone::Attention,
            20.0 * MINUTE,
        ),
        (
            "s3://my-bucket",
            "user/qc-plates-2026-08",
            latest.0,
            latest.1,
            2.0 * HOUR,
        ),
        (
            "s3://my-bucket",
            "user/scratch",
            latest.0,
            latest.1,
            5.0 * HOUR,
        ),
        (
            "s3://my-bucket",
            "user/reference-genomes",
            latest.0,
            latest.1,
            26.0 * HOUR,
        ),
        (
            "s3://team-bucket",
            "team/rnaseq-batch-2026-07-31-reprocessed-v2",
            "No access",
            StateTone::Danger,
            3.0 * DAY,
        ),
        (
            "s3://team-bucket",
            "team/imaging-cohort-b",
            "No access",
            StateTone::Danger,
            9.0 * DAY,
        ),
        (
            "s3://org-archive",
            "org/dataset-c",
            latest.0,
            latest.1,
            21.0 * DAY,
        ),
        (
            "s3://org-archive",
            "org/dataset-d",
            latest.0,
            latest.1,
            70.0 * DAY,
        ),
        // Deliberately a `user/` package in someone else's bucket. Without one, every
        // prefix here would sit in exactly one bucket and the two axes would produce
        // the same groups with different names — which would hide the reason the
        // annotation exists on one axis and not the other.
        (
            "s3://org-archive",
            "user/shared-archive",
            latest.0,
            latest.1,
            11.0 * DAY,
        ),
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

/// Two spellings of the same header, because `GroupHeading`'s `annotation` is
/// `#[prop(optional, into)]` and Leptos makes that setter take `impl Into<String>`
/// — so an `Option<String>` cannot be forwarded to it. Harmless here, where the
/// condition is a fixture, but the real page reads the cause off a DTO as an
/// `Option` and will hit this. Noted rather than fixed: changing the prop would
/// cost every caller that passes a literal.
fn header(title: &'static str, count: usize, bucket_axis: bool) -> AnyView {
    // Only the bucket axis carries a cause. A prefix spans buckets — `user/` has
    // packages in two of them here — so no cause can be a property of a prefix group,
    // and the slot stays empty rather than repeating a per-row problem.
    if bucket_axis && title == "s3://team-bucket" {
        view! { <GroupHeading title=title count=count annotation="no access as analyst" /> }
            .into_any()
    } else {
        view! { <GroupHeading title=title count=count /> }.into_any()
    }
}

/// The namespace's owner segment, with its slash — `user/package-a` groups under
/// `user/`. Borrowed from a `&'static str`, so the group key needs no allocation.
fn prefix(namespace: &'static str) -> &'static str {
    namespace
        .split_once('/')
        .map_or(namespace, |(owner, _)| &namespace[..=owner.len()])
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

/// The region itself, so the whole-page scene composes this code rather than a copy.
#[component]
pub fn PackagesRegion() -> impl IntoView {
    let view_mode = RwSignal::new("Packages".to_string());
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Recently changed".to_string());

    view! {
        // No title: the SegmentedControl names the view, and a card headed `Packages` above a
        // Packages / Recent files switch says it twice. One wrapper child, so the
        // card's between-children hairline does not double the rows' own.
        <Card>
            <div>
                <ListToolbar>
                    <SegmentedControl
                        aria_label="List view"
                        name="packages-view"
                        options=vec!["Packages".to_string(), "Recent files".to_string()]
                        selected=view_mode
                    />
                    <SearchInput value=query aria_label="Search packages" placeholder="Search…" />
                    <Select
                        naming=Naming::Prefix("Group".to_string())
                        options=vec![
                            "Bucket".to_string(),
                            "Prefix".to_string(),
                            "None".to_string(),
                        ]
                        selected=group
                    />
                    <Select
                        naming=Naming::Prefix("Sort".to_string())
                        options=vec!["Recently changed".to_string(), "Name".to_string()]
                        selected=sort
                    />
                    <Button variant=ButtonVariant::Primary on_click=|_| ()>
                        "Create package"
                    </Button>
                </ListToolbar>
                {move || {
                    let rows = fixtures();
                    let axis = group.get();
                    if axis == "None" {
                        return rows.into_iter().map(row).collect_view().into_any();
                    }
                    let bucket_axis = axis == "Bucket";
                    // One path for both axes, differing only in the key. Grouping by
                    // prefix used to fall through to the flat list, which read as
                    // "sorted by prefix" — the same rows in the same order with no
                    // headers at all.
                    let key = move |entry: &(&'static str, &'static str, &str, StateTone, f64)| {
                        if bucket_axis { entry.0 } else { prefix(entry.1) }
                    };
                    // First-appearance order, not sorted: the fixture order is the
                    // page's sort order, and re-sorting the groups here would hide
                    // whatever the Sort control did.
                    let mut order: Vec<&'static str> = Vec::new();
                    for entry in &rows {
                        let k = key(entry);
                        if !order.contains(&k) {
                            order.push(k);
                        }
                    }
                    order
                        .into_iter()
                        .map(|group_key| {
                            let group_rows: Vec<_> = rows
                                .iter()
                                .copied()
                                .filter(|entry| key(entry) == group_key)
                                .collect();
                            view! {
                                {header(group_key, group_rows.len(), bucket_axis)}
                                {group_rows.into_iter().map(row).collect_view()}
                            }
                                .into_any()
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </Card>
    }
}

#[component]
pub fn PackagesScene() -> impl IntoView {
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
                  the two views compose different controls. \
                  \
                  All three Group options are live. Switch between Bucket and Prefix and \
                  watch user/shared-archive move: it is a user/ package in s3://org-archive, \
                  so the two axes cut the same list differently. That is also why only the \
                  bucket axis annotates a group with a shared cause — a prefix spans \
                  buckets, so no cause can be a property of one."
        >
            <PackagesRegion />
        </Scene>
        <Scene
            title="Scene · a fresh install"
            note="Zero packages, which is the FIRST thing a new user sees and the one empty \
                  state that was missing — the other two are 'no results' for a search, \
                  which is a different situation with a different answer. \
                  \
                  This one carries an action, because the user genuinely cannot proceed \
                  without one and has no way to guess it. 'No results' carries none, \
                  because they already know how to change their own search."
        >
            <Card>
                <div>
                    <Blankslate
                        heading="No packages yet"
                        description="A package is a folder of files that QuiltSync keeps in step with \
                              an S3 bucket. Create one from a folder you already have, or \
                              install an existing package from the catalog."
                        primary_action=view! {
                            <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                "Create package"
                            </Button>
                        }
                            .into_any()
                    />
                </div>
            </Card>
        </Scene>
        <Scene
            title="Scene · packages, no results"
            note="One Blankslate component for both empties. No results carries no action — \
                  the user already knows how to change their own search, so a button here \
                  would be filler."
        >
            <Blankslate
                heading="No packages match “plate-99”"
                description="Search covers the names of packages installed on this machine."
            />
        </Scene>
    }
}
