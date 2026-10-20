//! Each package's autosync state, stacked on the Autosync card's publish toggle.
//!
//! Fake data only. The app's card still reads one deadline per toggle; these
//! scenes show what it says once the watcher sends a state per package.

use leptos::prelude::*;

use crate::Scene;
use crate::commands::PausedReasonData;
use crate::kit::Card;
use crate::kit::Countdown;
use crate::kit::ToggleRow;
use quilt_sync_ui::pages::StateStrip;
use quilt_sync_ui::pages::package_autosync::PackageAutosync;
use quilt_sync_ui::pages::package_autosync::PublishCard;
use quilt_sync_ui::pages::package_autosync::publish_detail;
use quilt_sync_ui::pages::package_autosync::publish_trailing;

const MINUTE: f64 = 60_000.0;
/// The wait in the screenshot this work started from.
const WAIT_MIN: f64 = 120.0;

fn in_min(minutes: f64) -> f64 {
    js_sys::Date::now() + minutes * MINUTE
}

fn waiting(minutes: f64) -> PackageAutosync {
    PackageAutosync::Waiting {
        publish_at: in_min(minutes),
        changed_while_closed: false,
    }
}

fn first_seen(minutes: f64) -> PackageAutosync {
    PackageAutosync::Waiting {
        publish_at: in_min(minutes),
        changed_while_closed: true,
    }
}

fn retrying() -> PackageAutosync {
    PackageAutosync::Retrying {
        error: "Could not reach S3".to_string(),
        next_try_at: in_min(4.0),
    }
}

/// One fake package and what autosync is doing with it.
#[derive(Clone)]
struct Pkg {
    namespace: &'static str,
    autosync: PackageAutosync,
}

fn pkg(namespace: &'static str, autosync: PackageAutosync) -> Pkg {
    Pkg {
        namespace,
        autosync,
    }
}

fn latest(namespace: &'static str) -> Pkg {
    pkg(namespace, PackageAutosync::NoChanges)
}

/// The card on fake packages. The publish toggle is live: turning it off shows
/// `Off`.
#[component]
fn PerPackage(packages: Vec<Pkg>, #[prop(optional)] off: bool) -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(!off);
    let named: Vec<(String, PackageAutosync)> = packages
        .iter()
        .map(|p| (p.namespace.to_string(), p.autosync.clone()))
        .collect();
    let card = Memo::new(move |_| PublishCard::of(publish.get(), &named));
    let title = format!("Publishes after {WAIT_MIN} min of inactivity");

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
                        {move || publish_trailing(&card.get(), WAIT_MIN * MINUTE, &title)}
                    }
                        .into_any()
                    detail=view! { {move || publish_detail(&card.get())} }.into_any()
                />
            </Card>
        </StateStrip>
    }
}

/// The screenshot this work started from: five packages with changes.
fn screenshot() -> Vec<Pkg> {
    vec![
        pkg("user/package-a", waiting(118.0)),
        pkg("user/package-b", waiting(97.0)),
        pkg("user/qc-plates-2026-08", waiting(64.0)),
        pkg("team/rnaseq-batch-2026-07-31", waiting(110.0)),
        pkg("org/dataset-c", waiting(81.0)),
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
                  card that said nothing to publish. Now each package with changes has its own \
                  ring, soonest first; past three, a plain label counts the rest. No clock times: a ring \
                  shows how far along a wait is without promising a minute."
        >
            <PerPackage packages=screenshot() />
        </Scene>
        <Scene
            title="Scene · three packages with changes"
            note="One ring per package, in the slot the single ring used, each with its \
                  package's name to the left as a hint. Soonest first."
        >
            <PerPackage packages=vec![
                pkg("user/package-a", waiting(20.0)),
                pkg("user/package-b", waiting(55.0)),
                pkg("org/dataset-c", waiting(90.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · publishing turned off during a countdown"
            note="The card says Off, not nothing to publish. Turn the toggle on to see \
                  the rings come back."
        >
            <PerPackage
                off=true
                packages=vec![
                    pkg("user/package-a", waiting(20.0)),
                    pkg("user/package-b", waiting(55.0)),
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
            note="The paused package sits in the stack with a Paused label; hover it for \
                  why and the fix. The others keep their rings. \"Needs your attention\" is \
                  unchanged and still carries the conflict."
        >
            <PerPackage packages=vec![
                pkg("user/package-a", waiting(20.0)),
                pkg(
                    "org/dataset-c",
                    PackageAutosync::Paused {
                        reason: PausedReasonData::PullConflict {
                            files: vec!["a.csv".to_string(), "b.csv".to_string()],
                        },
                    },
                ),
                pkg("user/package-b", waiting(55.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · publishing fails on a network error"
            note="Retrying in the package's place in the stack; hover it for the error."
        >
            <PerPackage packages=vec![
                pkg("user/package-a", retrying()),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · reopened with changes"
            note="Two packages changed while QuiltSync was closed, so each waits a full wait \
                  from launch, and a line under the toggle says so. A package edited since \
                  launch waits as usual."
        >
            <PerPackage packages=vec![
                pkg("user/package-a", first_seen(WAIT_MIN - 1.0)),
                pkg("user/package-b", first_seen(WAIT_MIN - 1.0)),
                pkg("org/dataset-c", waiting(30.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · publishing now"
            note="A spinner while a package publishes; the others still count down."
        >
            <PerPackage packages=vec![
                pkg("user/package-a", PackageAutosync::Publishing),
                pkg("user/package-b", waiting(55.0)),
                latest("user/scratch"),
            ] />
        </Scene>
        <Scene
            title="Scene · every kind at once"
            note="Publishing first, then the waits, then the troubles. Four packages, so \
                  the paused one is past the cap and counted in the label under the stack."
        >
            <PerPackage packages=vec![
                pkg("user/package-a", waiting(20.0)),
                pkg("user/package-b", PackageAutosync::Publishing),
                pkg("user/package-e", retrying()),
                pkg(
                    "team/imaging-cohort-b",
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
