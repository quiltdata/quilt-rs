//! Option C · Settings Storage section — an exploration for removing old revisions, not a shipped design.
//!
//! # Where it lives
//!
//! Domain-wide, in Settings, as a `Storage` section laid out the way the other
//! sections are (`src/pages/settings/*.rs`): a titled block holding a label
//! column and a value column. Disk is a property of *this computer*, not of one
//! package, so the question "what can I free?" is answered once for every
//! package rather than on each package's page.
//!
//! # Two rows, one sweep
//!
//! `Unused files` is the neighbouring Clean up — the parallel effort's button,
//! which frees stored data no installed revision uses and runs at once with no
//! confirmation. `Old revisions` is this option. Removing a revision deletes its
//! installed manifest and *then runs that same sweep*, which is where the bytes
//! actually come from — so the two rows sit together and the second says it
//! ends with the first. Clean up alone frees nothing an old revision still uses.
//!
//! # Sizes are for sets
//!
//! A package's `frees …` is what removing *all* its old revisions frees, and the
//! summary is what removing every package's frees. Both are computed for the
//! set by the backend, never added up here: objects shared only among the
//! removed revisions are freed too, so a set frees at least the sum of its rows.
//! The stub below sums its fixture only because it has no backend to ask.
//!
//! # Busy packages are skipped, not waited for
//!
//! A package under its per-package lock (syncing) cannot lose a manifest. Remove
//! all skips it and says so; its own Remove is disabled while the label stands.
//! Clean up, by contrast, stops outright when any package is locked — the two
//! behaviours differ, and the last cell draws both side by side.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::gallery::forms::after_a_beat;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::ConfirmDialog;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::RevisionRow;
use crate::kit::SkeletonBox;
use crate::kit::SkeletonText;
use crate::kit::Spinner;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::Submit;
use crate::kit::ZeroLine;
use crate::kit::icons;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// One revision this computer has. `kept` is the reason it is protected — a
/// protected row never offers removal, it says why.
struct Rev {
    message: &'static str,
    obtained: f64,
    published: bool,
    kept: Option<&'static str>,
    /// What removing this one alone frees. Honest about small numbers.
    frees: &'static str,
}

const fn rev(message: &'static str, obtained: f64, published: bool, frees: &'static str) -> Rev {
    Rev {
        message,
        obtained,
        published,
        kept: None,
        frees,
    }
}

const fn kept(message: &'static str, obtained: f64, published: bool, why: &'static str) -> Rev {
    Rev {
        message,
        obtained,
        published,
        kept: Some(why),
        frees: "",
    }
}

/// A package with revisions this computer has. `old` counts the removable ones.
struct Pkg {
    namespace: &'static str,
    old: usize,
    /// What removing every old revision frees, for the set, in MB. The stub's
    /// stand-in for the backend's answer; `< 1 MB` is stored as 0.4.
    frees_mb: f64,
    busy: bool,
    revs: fn() -> Vec<Rev>,
    /// A line under the opened list, where the order needs explaining.
    note: Option<&'static str>,
}

/// Every package, largest saving first — the order someone short of disk wants.
/// `team/notes` has nothing to remove and is folded into one muted line.
const PKGS: [Pkg; 5] = [
    Pkg {
        namespace: "lab/imaging",
        old: 5,
        frees_mb: 140.0,
        busy: true,
        revs: imaging,
        note: None,
    },
    Pkg {
        namespace: "user/plate-06",
        old: 3,
        frees_mb: 22.0,
        busy: false,
        revs: plate_06,
        note: None,
    },
    Pkg {
        namespace: "user/plate-07",
        old: 4,
        frees_mb: 6.9,
        busy: false,
        revs: plate_07,
        note: Some(
            "Together these free 6.9 MB — more than the rows add up to, because some files \
             are shared only among them. \u{201c}Add plate 6 controls\u{201d} was fetched \
             again yesterday, so it sits above a revision made after it.",
        ),
    },
    Pkg {
        namespace: "lab/assays",
        old: 1,
        frees_mb: 0.4,
        busy: false,
        revs: assays,
        note: None,
    },
    Pkg {
        namespace: "team/notes",
        old: 0,
        frees_mb: 0.0,
        busy: false,
        revs: notes,
        note: None,
    },
];

const PLATE_07: usize = 2;

/// The shared fixture, newest obtained first.
fn plate_07() -> Vec<Rev> {
    vec![
        kept(
            "Normalize well IDs",
            ago(20.0 * MINUTE),
            false,
            "current · not pushed",
        ),
        kept(
            "Re-run plate 7 with the corrected layout",
            ago(2.0 * HOUR),
            true,
            "latest · base",
        ),
        rev("Add plate 6 controls", ago(DAY), true, "frees 0.2 MB"),
        rev(
            "Add Caihong folder-upload note",
            ago(3.0 * DAY),
            true,
            "frees 1.2 MB",
        ),
        rev("Initial upload", ago(12.0 * DAY), true, "frees 4.8 MB"),
        rev("", ago(20.0 * DAY), true, "frees nothing"),
    ]
}

fn plate_06() -> Vec<Rev> {
    vec![
        kept(
            "Add the reader's calibration run",
            ago(HOUR),
            false,
            "current · not pushed",
        ),
        kept("Plate 6 final layout", ago(DAY), true, "latest · base"),
        rev("Swap wells B3 and B4", ago(4.0 * DAY), true, "frees 9.6 MB"),
        rev("Second read", ago(9.0 * DAY), true, "frees 8.1 MB"),
        rev("Initial upload", ago(30.0 * DAY), true, "frees 3.4 MB"),
    ]
}

fn assays() -> Vec<Rev> {
    vec![
        kept(
            "Q3 assay summary",
            ago(2.0 * DAY),
            true,
            "current · latest · base",
        ),
        rev("Draft summary", ago(15.0 * DAY), true, "frees < 1 MB"),
    ]
}

fn imaging() -> Vec<Rev> {
    vec![
        kept(
            "Add October scans",
            ago(10.0 * MINUTE),
            true,
            "current · latest · base",
        ),
        rev(
            "Re-export at full depth",
            ago(2.0 * DAY),
            true,
            "frees 62 MB",
        ),
        rev("September scans", ago(6.0 * DAY), true, "frees 41 MB"),
        rev(
            "Crop to the plate area",
            ago(11.0 * DAY),
            true,
            "frees 20 MB",
        ),
        rev("August scans", ago(25.0 * DAY), true, "frees 11 MB"),
        rev("Rename channels", ago(40.0 * DAY), true, "frees < 1 MB"),
    ]
}

fn notes() -> Vec<Rev> {
    vec![
        kept(
            "Meeting notes, 29 Sep",
            ago(3.0 * HOUR),
            false,
            "current · not pushed",
        ),
        kept(
            "Meeting notes, 22 Sep",
            ago(8.0 * DAY),
            true,
            "latest · base",
        ),
    ]
}

/// A package with one revision, for the only-the-current cell. Not part of the
/// domain above, so the summary's numbers stay the fixture's.
fn scratch() -> Vec<Rev> {
    vec![kept(
        "Try a new layout",
        ago(5.0 * HOUR),
        false,
        "current · not pushed",
    )]
}

fn megabytes(mb: f64) -> String {
    if mb < 1.0 {
        "< 1 MB".to_string()
    } else if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

fn revisions(n: usize) -> String {
    if n == 1 {
        "1 old revision".to_string()
    } else {
        format!("{n} old revisions")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    One(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Sizes are being measured. Clean up does not wait for them.
    Loading,
    Ready,
    Confirm(Scope),
    Working(Scope),
    Done(Scope),
    /// Removal stopped part-way: plate-07 went, plate-06 refused.
    Failed,
    /// No package has anything to free — every old revision already went.
    Empty,
    /// Clean up refused because a package is locked.
    CleanupStopped,
}

/// Whether `i`'s old revisions are gone, as of `stage`.
fn removed(stage: Stage, i: usize) -> bool {
    match stage {
        Stage::Done(Scope::All) => !PKGS[i].busy,
        Stage::Done(Scope::One(j)) => i == j,
        Stage::Failed => i == PLATE_07,
        Stage::Empty => true,
        _ => false,
    }
}

/// Indices of the packages with something left to free.
fn pending(stage: Stage) -> Vec<usize> {
    (0..PKGS.len())
        .filter(|&i| PKGS[i].old > 0 && !removed(stage, i))
        .collect()
}

fn set_mb(set: &[usize]) -> f64 {
    set.iter().map(|&i| PKGS[i].frees_mb).sum()
}

fn set_revisions(set: &[usize]) -> usize {
    set.iter().map(|&i| PKGS[i].old).sum()
}

/// The confirmation's one sentence. Every removable revision is published by
/// rule, so the second clause is always true.
fn consequence(scope: Scope) -> String {
    match scope {
        Scope::All => {
            let set = pending(Stage::Ready);
            format!(
                "Removes {} old revisions from this computer across {} packages and frees about \
                 {}. Published ones can be installed again from the remote.",
                set_revisions(&set),
                set.len(),
                megabytes(set_mb(&set)),
            )
        }
        Scope::One(i) => format!(
            "Removes {} of {} from this computer and frees about {}. Published ones can be \
             installed again from the remote.",
            revisions(PKGS[i].old),
            PKGS[i].namespace,
            megabytes(PKGS[i].frees_mb),
        ),
    }
}

/// The result line. The busy package is named with what to do about it.
fn result(scope: Scope) -> (BannerVariant, String) {
    match scope {
        Scope::All => {
            let done: Vec<usize> = pending(Stage::Ready)
                .into_iter()
                .filter(|&i| !PKGS[i].busy)
                .collect();
            let skipped: Vec<&str> = PKGS
                .iter()
                .filter(|p| p.busy && p.old > 0)
                .map(|p| p.namespace)
                .collect();
            let head = format!(
                "Removed {} revisions · freed {}.",
                set_revisions(&done),
                megabytes(set_mb(&done)),
            );
            if skipped.is_empty() {
                (BannerVariant::Success, head)
            } else {
                (
                    BannerVariant::Warning,
                    format!(
                        "{head} Skipped {} (busy syncing) — try again in a moment.",
                        skipped.join(", ")
                    ),
                )
            }
        }
        Scope::One(i) => (
            BannerVariant::Success,
            format!(
                "Removed {} revisions from {} · freed {}.",
                PKGS[i].old,
                PKGS[i].namespace,
                megabytes(PKGS[i].frees_mb),
            ),
        ),
    }
}

const FAILURE: &str = "Removed 4 revisions from user/plate-07 · freed 6.9 MB, then stopped: \
                       user/plate-06 could not be changed (permission denied). Nothing else was \
                       touched.";

const CLEANUP_STOPPED: &str =
    "Clean up did not run: lab/imaging is busy syncing — try again in a moment.";

/// A running removal, as the live cell plays it.
fn run(stage: RwSignal<Stage>, scope: Scope) {
    stage.set(Stage::Working(scope));
    leptos::task::spawn_local(async move {
        after_a_beat(1200).await;
        stage.set(Stage::Done(scope));
    });
}

/// The confirmation, inline at the modal's width — a live modal would take the
/// gallery's pointer events. The real one is a `ConfirmDialog` with this
/// sentence and `Remove` on the Danger button.
fn confirm_inline(stage: RwSignal<Stage>, scope: Scope) -> AnyView {
    view! {
        <div class="g-bars g-dialog-inline g-ors-dialog">
            <p class="g-ors-dialog__title">"Remove old revisions"</p>
            <p class="g-consequence">{consequence(scope)}</p>
            <div class="g-inline g-inline--end">
                <Button on_click=move |_| stage.set(Stage::Ready)>"Cancel"</Button>
                <Button variant=ButtonVariant::Danger on_click=move |_| run(stage, scope)>
                    "Remove"
                </Button>
            </div>
        </div>
    }
    .into_any()
}

/// The opened list: every revision the package has, newest obtained first,
/// protected ones in the muted level with their reason and no control.
fn detail(pkg: &Pkg) -> AnyView {
    let rows = (pkg.revs)()
        .into_iter()
        .map(|r| {
            let class = if r.kept.is_some() {
                "g-ors-rev g-ors-rev--kept"
            } else {
                "g-ors-rev"
            };
            let side = r
                .kept
                .map_or_else(|| r.frees.to_string(), |why| format!("kept — {why}"));
            view! {
                <li class=class>
                    <div class="g-ors-rev__row">
                        <RevisionRow message=r.message at=r.obtained published=r.published />
                    </div>
                    <span class="g-ors-rev__side">{side}</span>
                </li>
            }
        })
        .collect_view();
    view! {
        <div class="g-ors-detail">
            <p class="g-ors-muted">"Newest obtained first"</p>
            <ul class="g-ors-revs">{rows}</ul>
            {pkg.note.map(|n| view! { <p class="g-ors-muted">{n}</p> })}
        </div>
    }
    .into_any()
}

/// A package with something to free: the disclosure, the namespace, what it
/// frees, its lock if it has one, and its own Remove.
fn package_row(stage: RwSignal<Stage>, open: RwSignal<Option<usize>>, i: usize) -> AnyView {
    let pkg = &PKGS[i];
    let now = stage.get();
    let is_open = open.get() == Some(i);
    let working = matches!(now, Stage::Working(_));
    let label = format!(
        "{} {}",
        if is_open {
            "Hide revisions of"
        } else {
            "Show revisions of"
        },
        pkg.namespace
    );
    let chevron = if is_open {
        icons::chevron_down()
    } else {
        icons::chevron_right()
    };
    view! {
        <li class="g-ors-pkg">
            <div class="g-ors-pkg__line">
                <IconButton
                    icon=chevron
                    aria_label=label
                    variant=IconButtonVariant::Invisible
                    aria_expanded=is_open
                    on_click=move |_| open.update(|o| *o = if *o == Some(i) { None } else { Some(i) })
                />
                <span class="g-ors-pkg__name" title=pkg.namespace>{pkg.namespace}</span>
                <span class="g-ors-muted g-ors-pkg__frees">
                    {format!("{} · frees {}", revisions(pkg.old), megabytes(pkg.frees_mb))}
                </span>
                {pkg.busy.then(|| view! { <StateLabel tone=StateTone::Neutral>"busy — syncing"</StateLabel> })}
                <span class="g-ors-pkg__action">
                    <Button
                        disabled=pkg.busy || working
                        on_click=move |_| stage.set(Stage::Confirm(Scope::One(i)))
                    >
                        "Remove"
                    </Button>
                </span>
            </div>
            {(now == Stage::Confirm(Scope::One(i))).then(|| confirm_inline(stage, Scope::One(i)))}
            {is_open.then(|| detail(pkg))}
        </li>
    }
    .into_any()
}

/// The packages with nothing to free, folded into one muted line.
fn nothing_line(stage: Stage) -> Option<AnyView> {
    // The zero line has already said it for every package.
    if pending(stage).is_empty() {
        return None;
    }
    let idle: Vec<&str> = (0..PKGS.len())
        .filter(|&i| PKGS[i].old == 0 || removed(stage, i))
        .map(|i| PKGS[i].namespace)
        .collect();
    match idle.len() {
        0 => None,
        1 => Some(
            view! { <p class="g-ors-muted g-ors-idle">{format!("{}: nothing to remove", idle[0])}</p> }
                .into_any(),
        ),
        n => {
            let names = idle.join(", ");
            Some(
                view! {
                    <p class="g-ors-muted g-ors-idle" title=names.clone()>
                        {format!("{n} packages have nothing to remove: {names}")}
                    </p>
                }
                .into_any(),
            )
        }
    }
}

fn old_revisions(stage: RwSignal<Stage>, open: RwSignal<Option<usize>>) -> AnyView {
    let now = stage.get();
    if now == Stage::Loading {
        return view! {
            <div class="g-ors-value" aria-busy="true">
                <p class="g-ors-summary"><SkeletonText width="320px" /></p>
                <ul class="g-ors-pkgs">
                    <li class="g-ors-pkg g-ors-pkg--skeleton"><SkeletonBox width="360px" /></li>
                    <li class="g-ors-pkg g-ors-pkg--skeleton"><SkeletonBox width="300px" /></li>
                    <li class="g-ors-pkg g-ors-pkg--skeleton"><SkeletonBox width="330px" /></li>
                </ul>
                <span data-sr-only>"Measuring old revisions"</span>
            </div>
        }
        .into_any();
    }

    let set = pending(now);
    let working = matches!(now, Stage::Working(_));
    let all_busy = set.iter().all(|&i| PKGS[i].busy);
    let summary = if set.is_empty() {
        view! { <ZeroLine text=format!("No old revisions to remove in {} packages", PKGS.len()) /> }
            .into_any()
    } else if working {
        view! {
            <p class="g-ors-summary g-ors-working">
                <Spinner />
                "Removing old revisions, then freeing the files nothing else uses…"
            </p>
        }
        .into_any()
    } else {
        let place = if set.len() == 1 {
            format!("in {}", PKGS[set[0]].namespace)
        } else {
            format!("across {} packages", set.len())
        };
        view! {
            <p class="g-ors-summary">
                {format!("Old revisions: {} could be freed {place}", megabytes(set_mb(&set)))}
            </p>
        }
        .into_any()
    };

    let banner = match now {
        Stage::Done(scope) => {
            let (variant, text) = result(scope);
            Some(view! { <Banner variant=variant>{text}</Banner> }.into_any())
        }
        Stage::Failed => {
            Some(view! { <Banner variant=BannerVariant::Critical>{FAILURE}</Banner> }.into_any())
        }
        _ => None,
    };

    let rows = set
        .iter()
        .map(|&i| package_row(stage, open, i))
        .collect_view();

    view! {
        <div class="g-ors-value">
            {banner}
            <div class="g-ors-head">
                {summary}
                {(!set.is_empty()).then(|| view! {
                    <Button
                        loading=working
                        disabled=working || all_busy
                        on_click=move |_| stage.set(Stage::Confirm(Scope::All))
                    >
                        "Remove all old revisions"
                    </Button>
                })}
            </div>
            {(now == Stage::Confirm(Scope::All)).then(|| confirm_inline(stage, Scope::All))}
            <ul class="g-ors-pkgs">{rows}</ul>
            {nothing_line(now)}
            <p class="g-ors-muted">
                "Keeps the current revision, the base, the latest, anything not pushed and \
                 anything only on this computer. Removing ends with Clean up, which is what \
                 frees the files."
            </p>
        </div>
    }
    .into_any()
}

/// The Settings section itself. The page's own classes are not in the gallery's
/// stylesheet, so `_old_revisions_settings.scss` restates the section's look.
#[component]
fn StorageSection(stage: RwSignal<Stage>, open: RwSignal<Option<usize>>) -> impl IntoView {
    let cleanup_off = move || matches!(stage.get(), Stage::Working(_));
    view! {
        <section class="g-ors-section">
            <h2 class="g-ors-title">"Storage"</h2>
            <dl class="g-ors-list">
                <dt>"Unused files"</dt>
                <dd>
                    <div class="g-ors-value">
                        <div class="g-ors-head">
                            <p class="g-ors-summary g-ors-muted">
                                "Frees stored data no revision on this computer uses, empties the \
                                 manifest cache and removes stale staging folders."
                            </p>
                            <Button
                                disabled=Signal::derive(cleanup_off)
                                on_click=move |_| stage.set(Stage::CleanupStopped)
                            >
                                "Clean up"
                            </Button>
                        </div>
                        {move || (stage.get() == Stage::CleanupStopped).then(|| view! {
                            <Banner variant=BannerVariant::Warning>{CLEANUP_STOPPED}</Banner>
                        })}
                    </div>
                </dd>

                <dt>"Old revisions"</dt>
                <dd>{move || old_revisions(stage, open)}</dd>
            </dl>
        </section>
    }
}

/// A cell holding the section in one state.
fn staged(stage: Stage, open: Option<usize>) -> impl IntoView {
    view! { <StorageSection stage=RwSignal::new(stage) open=RwSignal::new(open) /> }
}

/// A single package's opened list, drawn alone — the folded line's contents for
/// the cells about packages with nothing to free.
fn lone(namespace: &'static str, revs: fn() -> Vec<Rev>, why: &'static str) -> AnyView {
    let pkg = Pkg {
        namespace,
        old: 0,
        frees_mb: 0.0,
        busy: false,
        revs,
        note: Some(why),
    };
    view! {
        <div class="g-ors-lone">
            <p class="g-ors-muted g-ors-idle">{format!("{namespace}: nothing to remove")}</p>
            {detail(&pkg)}
        </div>
    }
    .into_any()
}

#[component]
pub fn OldRevisionsSettingsScene() -> impl IntoView {
    let live_stage = RwSignal::new(Stage::Ready);
    let live_open = RwSignal::new(None);
    let real = RwSignal::new(false);

    view! {
        <Scene
            title="Option C · Settings Storage section"
            note="Domain-wide: one Storage section in Settings answers what this computer can \
                  free, across every package. Clean up sits above as its neighbour; removing \
                  old revisions deletes their manifests and then runs that sweep. Sizes are \
                  what a set frees, never a revision's full size. The first cell is live — \
                  open a package, Remove, confirm, and watch it run."
        >
            <Cell full=true label="live — try it">
                <StorageSection stage=live_stage open=live_open />
                <div class="g-inline">
                    <Button on_click=move |_| {
                        live_stage.set(Stage::Ready);
                        live_open.set(None);
                    }>"Reset"</Button>
                </div>
            </Cell>
            <Cell full=true label="loading — skeleton while sizes are measured; Clean up does not wait">
                {staged(Stage::Loading, None)}
            </Cell>
            <Cell full=true label="the ordinary case — 169 MB across 4 packages, lab/imaging busy">
                {staged(Stage::Ready, None)}
            </Cell>
            <Cell full=true label="user/plate-07 opened — obtained order, per-row frees, kept rows muted">
                {staged(Stage::Ready, Some(PLATE_07))}
            </Cell>
            <Cell full=true label="confirm — Remove all old revisions">
                {staged(Stage::Confirm(Scope::All), None)}
                <div class="g-inline">
                    <Button on_click=move |_| real.set(true)>"Open the real one"</Button>
                </div>
                <ConfirmDialog
                    open=real
                    title="Remove old revisions"
                    consequence=consequence(Scope::All)
                    confirm=Submit::new(
                        "Remove",
                        || async {
                            after_a_beat(700).await;
                            Ok(())
                        },
                    )
                />
            </Cell>
            <Cell full=true label="confirm — one package's Remove, under its row">
                {staged(Stage::Confirm(Scope::One(PLATE_07)), None)}
            </Cell>
            <Cell full=true label="working — spinner; every Remove and Clean up held">
                {staged(Stage::Working(Scope::All), None)}
            </Cell>
            <Cell full=true label="result — busy lab/imaging skipped and named">
                {staged(Stage::Done(Scope::All), None)}
            </Cell>
            <Cell full=true label="result — one package">
                {staged(Stage::Done(Scope::One(PLATE_07)), None)}
            </Cell>
            <Cell full=true label="nothing to free anywhere — zero line; Clean up still available">
                {staged(Stage::Empty, None)}
            </Cell>
            <Cell full=true label="failure — stopped part-way, says what went and what refused">
                {staged(Stage::Failed, None)}
            </Cell>
            <Cell full=true label="Clean up beside it — stops when a package is busy">
                {staged(Stage::CleanupStopped, None)}
            </Cell>
            <Cell wide=true label="only the current revision — what the folded line holds">
                {lone(
                    "user/scratch",
                    scratch,
                    "Its only revision is the current one.",
                )}
            </Cell>
            <Cell wide=true label="everything protected — team/notes">
                {lone(
                    "team/notes",
                    notes,
                    "Both revisions are kept: one is current and not pushed, the other is the latest.",
                )}
            </Cell>
        </Scene>
    }
}
