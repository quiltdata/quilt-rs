//! Each package's autosync state: on its rows, and added up on the Autosync card.
//!
//! Fake data only. The app's card still reads one deadline per toggle; these
//! scenes show what it says once the watcher sends a state per package.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::commands::PausedReasonData;
use crate::kit::Card;
use crate::kit::Countdown;
use crate::kit::PackageRow;
use crate::kit::PackageState;
use crate::kit::QueueRow;
use crate::kit::Remedy;
use crate::kit::Site;
use crate::kit::ToggleRow;
use crate::kit::render;
use quilt_sync_ui::pages::StateStrip;
use quilt_sync_ui::pages::action_href;
use quilt_sync_ui::pages::package_autosync::PackageAutosync;
use quilt_sync_ui::pages::package_autosync::PublishTotals;
use quilt_sync_ui::pages::package_autosync::publish_detail;
use quilt_sync_ui::pages::package_autosync::publish_trailing;
use quilt_sync_ui::pages::package_autosync::row_note_view;

const MINUTE: f64 = 60_000.0;
/// The wait in the screenshot this work started from.
const WAIT_MIN: f64 = 120.0;

fn in_min(minutes: f64) -> f64 {
    js_sys::Date::now() + minutes * MINUTE
}

fn waiting(minutes: f64) -> PackageAutosync {
    PackageAutosync::Waiting {
        publish_at: in_min(minutes),
        first_seen_at_start: false,
    }
}

fn first_seen(minutes: f64) -> PackageAutosync {
    PackageAutosync::Waiting {
        publish_at: in_min(minutes),
        first_seen_at_start: true,
    }
}

fn retrying() -> PackageAutosync {
    PackageAutosync::Retrying {
        error: "Could not reach S3".to_string(),
        next_try_at: in_min(4.0),
    }
}

/// One fake package: what its rows say, and what autosync is doing with it.
#[derive(Clone)]
struct Pkg {
    namespace: &'static str,
    state: PackageState,
    autosync: PackageAutosync,
}

fn pkg(namespace: &'static str, state: PackageState, autosync: PackageAutosync) -> Pkg {
    Pkg {
        namespace,
        state,
        autosync,
    }
}

fn changed(namespace: &'static str, files: usize, autosync: PackageAutosync) -> Pkg {
    pkg(namespace, PackageState::PendingChanges { files }, autosync)
}

fn latest(namespace: &'static str) -> Pkg {
    pkg(namespace, PackageState::Latest, PackageAutosync::NoChanges)
}

/// With publishing off, every package with changes says so.
fn shown(publishing: bool, state: &PackageAutosync) -> PackageAutosync {
    match state {
        PackageAutosync::NoChanges => PackageAutosync::NoChanges,
        _ if !publishing => PackageAutosync::Off,
        other => other.clone(),
    }
}

fn list_row(pkg: &Pkg, publishing: bool) -> AnyView {
    let rendered = render(&pkg.state, Site::ListRow);
    view! {
        <PackageRow
            namespace=pkg.namespace
            href="#package-autosync"
            changed_at=in_min(-12.0)
            state=rendered.words
            tone=rendered.tone
            note=row_note_view(&shown(publishing, &pkg.autosync))
        />
    }
    .into_any()
}

fn queue_row(pkg: &Pkg, publishing: bool) -> AnyView {
    let rendered = render(&pkg.state, Site::QueueRow);
    let namespace = quilt_uri::Namespace::try_from(pkg.namespace).expect("a scene namespace");
    let remedy = rendered.action.map(|action| Remedy {
        action,
        href: action_href(action, &namespace),
    });
    view! {
        <QueueRow
            namespace=pkg.namespace
            state=rendered.words
            tone=rendered.tone
            remedy=remedy
            note=row_note_view(&shown(publishing, &pkg.autosync))
        />
    }
    .into_any()
}

#[component]
pub fn RowNoteStories() -> impl IntoView {
    let cells: Vec<(&'static str, Pkg)> = vec![
        ("no changes — no line", latest("user/package-a")),
        ("waiting", changed("user/package-b", 2, waiting(42.0))),
        (
            "waiting, first seen at start",
            changed("user/package-c", 5, first_seen(WAIT_MIN)),
        ),
        (
            "publishing",
            changed("user/package-d", 3, PackageAutosync::Publishing),
        ),
        ("retrying", changed("user/package-e", 1, retrying())),
        (
            "paused — a conflict, with its fix",
            pkg(
                "org/dataset-c",
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string(), "b.csv".to_string()],
                },
                PackageAutosync::Paused {
                    reason: PausedReasonData::PullConflict {
                        files: vec!["a.csv".to_string(), "b.csv".to_string()],
                    },
                },
            ),
        ),
        (
            "paused — the engine's message, as is",
            pkg(
                "team/imaging-cohort-b",
                PackageState::Paused,
                PackageAutosync::Paused {
                    reason: PausedReasonData::Other {
                        message:
                            "package does not satisfy the workflow: a commit message is required"
                                .to_string(),
                    },
                },
            ),
        ),
        (
            "publishing off",
            changed("user/package-f", 2, PackageAutosync::Off),
        ),
    ];
    view! {
        <Story
            title="Package rows — autosync line"
            note="The line under a package's name says what autosync does with it. The \
                  row's own label still says what the package holds. Only a retry and a \
                  pause take the attention colour. Each state is shown on a list row, then \
                  on a \"Needs your attention\" row."
        >
            {cells
                .into_iter()
                .map(|(label, pkg)| {
                    view! {
                        <Cell full=true label=label>
                            <div class="g-rows">
                                {list_row(&pkg, true)}
                                {(pkg.autosync != PackageAutosync::NoChanges)
                                    .then(|| queue_row(&pkg, true))}
                            </div>
                        </Cell>
                    }
                })
                .collect_view()}
            <Cell wide=true label="narrow · a long line wraps under the name">
                <div class="g-rows">
                    {list_row(&changed("user/package-e", 1, retrying()), true)}
                    {list_row(
                        &changed(
                            "team/rnaseq-batch-2026-07-31-reprocessed-v2",
                            5,
                            first_seen(WAIT_MIN),
                        ),
                        true,
                    )}
                </div>
            </Cell>
        </Story>
    }
}

/// The card on fake packages, and the rows under it. The publish toggle is live:
/// turning it off shows `Off` and each row's `not published automatically`.
#[component]
fn PerPackage(packages: Vec<Pkg>, #[prop(optional)] off: bool) -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(!off);
    let states: Vec<PackageAutosync> = packages.iter().map(|p| p.autosync.clone()).collect();
    let totals = {
        let states = states.clone();
        Memo::new(move |_| PublishTotals::of(publish.get(), &states))
    };
    let title = format!("Publishes after {WAIT_MIN} min of inactivity");
    let queued: Vec<Pkg> = packages
        .iter()
        .filter(|p| p.autosync != PackageAutosync::NoChanges)
        .cloned()
        .collect();
    let queued_count = queued.len();

    view! {
        <StateStrip>
            <Card title="Autosync">
                <ToggleRow
                    label="Get new revisions"
                    sublabel="Every 3 min, keeping any local changes"
                    checked=pull
                    trailing=view! {
                        <Countdown
                            deadline=Signal::stored(Some(in_min(2.0)))
                            interval=3.0 * MINUTE
                            title="Checks for new revisions every 3 min"
                            repeat=true
                        />
                    }
                        .into_any()
                />
                <ToggleRow
                    label="Publish your changes"
                    sublabel=format!("After {WAIT_MIN} min of inactivity")
                    checked=publish
                    trailing=view! {
                        {move || publish_trailing(&totals.get(), WAIT_MIN * MINUTE, title.clone())}
                    }
                        .into_any()
                    detail=view! { {move || publish_detail(&totals.get())} }.into_any()
                />
            </Card>
        </StateStrip>
        {(queued_count > 0)
            .then(|| {
                let queued = queued.clone();
                view! {
                    <Card title="Needs your attention" count=queued_count>
                        <div>
                            {move || {
                                let on = publish.get();
                                queued.iter().map(|p| queue_row(p, on)).collect_view()
                            }}
                        </div>
                    </Card>
                }
            })}
        <div class="g-rows">
            {move || {
                let on = publish.get();
                packages.iter().map(|p| list_row(p, on)).collect_view()
            }}
        </div>
    }
}

/// The screenshot this work started from: five packages with changes.
fn screenshot() -> Vec<Pkg> {
    vec![
        changed("user/package-a", 2, waiting(118.0)),
        changed("user/package-b", 1, waiting(97.0)),
        changed("user/qc-plates-2026-08", 14, waiting(64.0)),
        changed("team/rnaseq-batch-2026-07-31", 3, waiting(110.0)),
        changed("org/dataset-c", 6, waiting(81.0)),
        latest("user/scratch"),
        latest("user/reference-genomes"),
    ]
}

#[component]
pub fn PerPackageScene() -> impl IntoView {
    view! { <CountingScenes /> <TroubleScenes /> }
}

#[component]
fn CountingScenes() -> impl IntoView {
    view! {
        <Scene
            title="Scene · five changed packages"
            note="The screenshot this work started from: five packages with changes and a \
                  card that said nothing to publish. Now the card counts them and names the \
                  next time, and each row says its own time."
        >
            <PerPackage packages=screenshot() />
        </Scene>
        <Scene
            title="Scene · three packages with changes"
            note="Each row shows its own time; the card shows how many wait and the \
                  earliest time. The ring counts down to that earliest time."
        >
            <PerPackage packages=vec![
                changed("user/package-a", 2, waiting(20.0)),
                changed("user/package-b", 1, waiting(55.0)),
                changed("org/dataset-c", 4, waiting(90.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · publishing turned off during a countdown"
            note="The card says Off, not nothing to publish, and each row with changes says \
                  it is not published automatically. Turn the toggle on to see the countdown \
                  this one left."
        >
            <PerPackage
                off=true
                packages=vec![
                    changed("user/package-a", 2, waiting(20.0)),
                    changed("user/package-b", 1, waiting(55.0)),
                    latest("user/scratch"),
                ]
            />
        </Scene>
    }
}

#[component]
fn TroubleScenes() -> impl IntoView {
    view! {
        <Scene
            title="Scene · one package can't publish"
            note="That row says paused and why, with its fix. The others keep counting, \
                  and the card counts them beside the paused one."
        >
            <PerPackage packages=vec![
                changed("user/package-a", 2, waiting(20.0)),
                pkg(
                    "org/dataset-c",
                    PackageState::PullConflict {
                        files: vec!["a.csv".to_string(), "b.csv".to_string()],
                    },
                    PackageAutosync::Paused {
                        reason: PausedReasonData::PullConflict {
                            files: vec!["a.csv".to_string(), "b.csv".to_string()],
                        },
                    },
                ),
                changed("user/package-b", 1, waiting(55.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · publishing fails on a network error"
            note="The card says Retrying; the row shows the error and the next try."
        >
            <PerPackage packages=vec![
                changed("user/package-a", 2, retrying()),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · reopened with changes"
            note="Two packages changed while QuiltSync was closed, so each waits a full wait \
                  from launch, and the card says so. A package edited since launch waits as \
                  usual."
        >
            <PerPackage packages=vec![
                changed("user/package-a", 2, first_seen(WAIT_MIN - 1.0)),
                changed("user/package-b", 7, first_seen(WAIT_MIN - 1.0)),
                changed("org/dataset-c", 1, waiting(30.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · publishing now"
            note="A spinner while a package publishes; the others still count down."
        >
            <PerPackage packages=vec![
                changed("user/package-a", 2, PackageAutosync::Publishing),
                changed("user/package-b", 1, waiting(55.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · every kind at once"
            note="Waiting, publishing, retrying and paused, counted in that order."
        >
            <PerPackage packages=vec![
                changed("user/package-a", 2, waiting(20.0)),
                changed("user/package-b", 1, PackageAutosync::Publishing),
                changed("user/package-e", 3, retrying()),
                pkg(
                    "team/imaging-cohort-b",
                    PackageState::Paused,
                    PackageAutosync::Paused {
                        reason: PausedReasonData::Other {
                            message: "package does not satisfy the workflow".to_string(),
                        },
                    },
                ),
            ] />
        </Scene>
        <Scene
            title="Scene · nothing changed"
            note="The only case that says nothing to publish."
        >
            <PerPackage packages=vec![latest("user/package-a"), latest("user/scratch")] />
        </Scene>
    }
}
