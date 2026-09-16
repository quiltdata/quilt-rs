//! Regions, stories and scenes for the installed-package page v2.
//!
//! Design: `proj/quilt-rs-tech-plans/2026-09-15-installed-package-page/`.
//!
//! # Why the regions live here
//!
//! Same reason `QueueRegion` does: the page does not exist yet, and this is where
//! it gets argued about before it is built. Each region below is composed from kit
//! components where the kit has one, and from local markup where it does not — the
//! design names three components the kit is missing (`EntryRow`, `FacetBar`,
//! `ChoiceRow`) plus a `GroupHeading` that collapses, and the mocks here are what
//! those are drawn from. When they land, these regions compose them instead and
//! the `g-ip-*` rules move into their stylesheets, exactly as `.g-strip`'s comment
//! anticipates for the state strip.
//!
//! # The bet this page makes
//!
//! Two panes, because `minWidth` is 1024 and `minHeight` is 560: horizontal room
//! is plentiful and vertical room is not, so everything that is not the file list
//! is spent sideways rather than stacked on top of it. The whole-page scene is
//! framed at the real height floor — count the rows that survive above the fold,
//! then compare with v1, which spends roughly 320px of that 560 on chrome.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::Naming;
use crate::kit::PageLayout;
use crate::kit::RelativeTime;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::icons;

const HOUR: f64 = 60.0 * 60.0 * 1000.0;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// Inline, like `file_row.rs`'s three: the kit's shared icon set has `gear` and
/// `sync` only, and an overflow glyph belongs to a real icon set rather than to
/// a gallery mock.
fn overflow_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
            <circle cx="3.5" cy="8" r="1.3" />
            <circle cx="8" cy="8" r="1.3" />
            <circle cx="12.5" cy="8" r="1.3" />
        </svg>
    }
    .into_any()
}

fn appbar_actions() -> AnyView {
    view! {
        <Button leading_visual=icons::sync() on_click=|_| ()>
            "Refresh"
        </Button>
        <Button leading_visual=icons::gear() on_click=|_| ()>
            "Settings"
        </Button>
    }
    .into_any()
}

#[component]
pub fn InstalledPackageStories() -> impl IntoView {
    view! {
        <HeaderStates />
        <DegradedBand />
        <RegionScenes />
        <WholePage />
        <NothingToSay />
    }
}

// ── The header's state band ──────────────────────────────────────────────────

/// One header, full width, in one state.
///
/// Full width and the real region, not a label-and-button pair in a grid cell:
/// the header's whole job is to hold identity, state and one action on one bar,
/// and a 280px cell cannot show whether that bar works. The longest label is the
/// one to watch — naming the host inside it is what breaks the line first.
fn header_cell(
    label: &'static str,
    state_label: &'static str,
    tone: StateTone,
    action: Option<&'static str>,
) -> AnyView {
    view! {
        <Cell label=label full=true>
            <div class="g-ip-headerframe">
                <PackageHeaderRegion
                    state_label=state_label
                    tone=tone
                    action=action.map(ToString::to_string)
                />
            </div>
        </Cell>
    }
    .into_any()
}

#[component]
fn HeaderStates() -> impl IntoView {
    view! {
        <Scene
            title="Package header · the settled states"
            note="Eight conditions, one resolved state each, at most one action — the real \
                  header at full width, once per state. Read the tones down the column: \
                  Success is the resting state and appears once; Neutral is a fact you may \
                  act on; Attention is waiting on you; Danger is something the page cannot \
                  fix by itself. No label may contain commit, push, pull, remote, behind, \
                  ahead, diverged or dirty — that rule is a test, not a convention, and \
                  this is where a violation is seen before the test is widened to cover it."
        >
            {header_cell("settled", "Latest", StateTone::Success, None)}
            {header_cell("local edits", "2 files changed", StateTone::Neutral, Some("Publish"))}
            {header_cell(
                "upstream moved",
                "Newer revision available",
                StateTone::Attention,
                Some("Get latest"),
            )}
            {header_cell(
                "committed, not sent",
                "Revision not published",
                StateTone::Attention,
                Some("Publish"),
            )}
            {header_cell(
                "bucket set, never sent",
                "Not published yet",
                StateTone::Attention,
                Some("Publish"),
            )}
            {header_cell(
                "no bucket chosen",
                "No S3 bucket yet",
                StateTone::Attention,
                Some("Choose S3 bucket"),
            )}
            {header_cell(
                "two-sided, working tree",
                "Conflicts in 2 files",
                StateTone::Danger,
                Some("Publish"),
            )}
            {header_cell(
                "two-sided, committed",
                "Changed in both places",
                StateTone::Danger,
                Some("Resolve"),
            )}
        </Scene>
    }
}

#[component]
fn DegradedBand() -> impl IntoView {
    view! {
        <Scene
            title="Package header · the degraded band"
            note="The four conditions that outrank every sync state, in precedence order. \
                  Three of them are one string today — status 'error', rendered as 'Unable \
                  to check remote status' — and the fourth is rendered as nothing at all: a \
                  credential S3 refuses is swallowed as if the remote were merely \
                  unreachable, so the page shows last-known state with no message \
                  (qhq-2apr). Telling them apart needs a field on the DTO, which is why \
                  they are drawn here before they exist. A denial does not offer Sign in: \
                  the session is healthy, and signing in again re-vends the same role."
        >
            {header_cell(
                "1 · no session for the host",
                "Signed out of demo.quiltdata.com",
                StateTone::Danger,
                Some("Sign in"),
            )}
            {header_cell("2 · credential refused", "Sign-in expired", StateTone::Danger, Some("Sign in"))}
            {header_cell(
                "3 · role cannot reach the bucket",
                "No access",
                StateTone::Danger,
                Some("Switch role"),
            )}
            {header_cell(
                "4 · genuinely offline",
                "Can't reach Quilt right now",
                StateTone::Danger,
                Some("Try again"),
            )}
        </Scene>
    }
}

// ── Region: the package header ───────────────────────────────────────────────

/// The header, so the whole-page scene composes this code rather than a copy.
///
/// Three lines: trail, identity, then one state and one action. Everything else
/// package-level is behind the overflow — `Open folder` is the one exception,
/// because it is the most-used escape hatch in a file-sync app and harmless.
#[component]
pub fn PackageHeaderRegion(
    #[prop(into)] state_label: String,
    tone: StateTone,
    /// A plain `Option`, with no `#[prop]` attribute: `optional` would make the
    /// macro expect the inner `String` at each call site, and every caller here
    /// already holds an `Option` because "this state has no action" is the point.
    action: Option<String>,
) -> impl IntoView {
    view! {
        <header class="g-ip-header">
            <a class="g-ip-trail" href="#installed-package-v2">"‹ Packages"</a>
            // One bar, not three stacked rows. The state qualifies the name, so it sits
            // beside it rather than on a line of its own with a gap to the controls —
            // which read as three unrelated rows. Two lines instead of three also returns
            // ~40px to the list, which is §9's first lever.
            //
            // The bucket moved to the context pane: it is a fact about the package, not
            // part of its identity or a decision, and the header is for identity, state
            // and the one action.
            <div class="g-ip-headerbar">
                <h3 class="g-ip-name">"proj/quilt-rs-feedback"</h3>
                <StateLabel tone=tone>{state_label}</StateLabel>
                <span class="g-ip-spacer" />
                {action
                    .map(|a| {
                        view! {
                            <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                {a}
                            </Button>
                        }
                    })}
                <Button on_click=|_| ()>"Open folder"</Button>
                <IconButton
                    icon=overflow_icon()
                    aria_label="More package actions"
                    variant=IconButtonVariant::Invisible
                    on_click=|_| ()
                />
            </div>
        </header>
    }
}

// ── Region: the file pane ────────────────────────────────────────────────────

/// One entry, plus the marker a pending decision puts on the rows it touches.
/// The marker is a word, not only the colour bar: a stripe says "this one is
/// special" and leaves the reader to guess which special thing it means.
fn entry_marked(
    name: &'static str,
    status: &'static str,
    size: &'static str,
    checked: bool,
    differs: bool,
) -> AnyView {
    // Only a row that can be downloaded carries a checkbox. A tick on a file that is
    // already here means nothing the footer could act on, and it is what made
    // "Select all 56" and "Download 17" disagree.
    let selectable = status == "Not downloaded";
    view! {
        <label
            class="g-ip-row"
            class:g-ip-row--differs=differs
            // Why the row is red, in one line. `title` is not a complete answer to the
            // colour-only problem — it is not keyboard-reachable and not there on touch —
            // but it is the cheap half, and it is the half a mouse user gets for free.
            // The non-colour cue for assistive tech is still owed; see the design's §5.
            title=differs
                .then_some(
                    "Your version of this file and the published version have different contents.",
                )
        >
            // Empty, and the same width as a group's disclosure button, so a file's
            // checkbox sits directly under its group's rather than a triangle's width
            // to the left of it.
            <span class="g-ip-gutter" />
            {if selectable {
                view! { <input type="checkbox" prop:checked=checked /> }.into_any()
            } else {
                view! { <span class="g-ip-row__nobox" /> }.into_any()
            }}
            <span class="g-ip-row__name">{name}</span>
            // No word. The rows are striped and the pane says "2 files differ between
            // these revisions — marked in the list"; a highlighted-rows-and-legend
            // pattern explains itself once instead of twenty times, and gives a busy
            // row back a column.
            //
            // Two words were tried and both failed on first reading — "differs on
            // Quilt" (which also broke principle 4, naming the platform inside an app
            // called QuiltSync) and "differs" alone. A sentence with room to explain
            // does not have to fit in two words. If a label is ever wanted back, the
            // thing it must convey is not how the two sides differ but that these are
            // the only files the choice touches.
            <span class="g-ip-row__state">
            {(status != "Downloaded" && status != "Ignored")
                .then(|| {
                    let tone = match status {
                        "Changed" | "New" => StateTone::Attention,
                        "Deleted" => StateTone::Danger,
                        _ => StateTone::Neutral,
                    };
                    view! { <StateLabel tone=tone>{status}</StateLabel> }
                })}
            </span>
            <span class="g-ip-row__size">{size}</span>
            <IconButton
                icon=overflow_icon()
                aria_label="More actions for this file"
                variant=IconButtonVariant::Invisible
                on_click=|_| ()
            />
        </label>
    }
    .into_any()
}

/// A package's files, as the DTO carries them: a full logical path, a state, a size.
/// Shaped after `proj/quilt-rs-feedback`, which is the degenerate case — 96% of its
/// files sit under one top-level directory.
const FILES: &[(&str, &str, &str)] = &[
    ("README.md", "Changed", "9.1 KB"),
    (".DS_Store", "Ignored", "6 KB"),
    (
        "feedback/2026-07-10-kevin-quiltsync-version-mismatch.md",
        "Downloaded",
        "5.4 KB",
    ),
    (
        "feedback/2026-07-11-ernest-autopush-stays-paused.md",
        "Downloaded",
        "4.9 KB",
    ),
    (
        "feedback/2026-07-11-ernest-autopush-stays-paused.diagnostic/README.md",
        "Downloaded",
        "1.2 KB",
    ),
    (
        "feedback/2026-07-11-ernest-autopush-stays-paused.diagnostic/logs/quilt-sync.log",
        "Downloaded",
        "88 KB",
    ),
    (
        "feedback/2026-07-15-ernest-quiltsync-pull-new-paths.md",
        "Changed",
        "10.6 KB",
    ),
    (
        "feedback/2026-08-03-ernest-main-page-attention-queue.md",
        "Not downloaded",
        "4.8 KB",
    ),
    (
        "feedback/2026-08-11-vir-caihong-folder-upload.md",
        "Not downloaded",
        "7.2 KB",
    ),
    (
        "feedback/2026-08-19-kevin-role-switch-notes.md",
        "Not downloaded",
        "3.1 KB",
    ),
];

/// A heading and the rows under it. Named because clippy is right that the tuple
/// is doing too much work inline.
type Group = (
    String,
    Vec<&'static (&'static str, &'static str, &'static str)>,
);

/// The paths the two revisions disagree about, in resolve mode. A named set and
/// not a flag on every row: the pane says "2 files differ" and a mock that striped
/// all ten would teach the reader that the marking means something else.
const DIVERGED: &[&str] = &[
    "feedback/2026-07-15-ernest-quiltsync-pull-new-paths.md",
    "README.md",
];

pub(super) const GROUP_FOLDER: &str = "Base folder";
pub(super) const GROUP_TOP: &str = "Top folder";
pub(super) const GROUP_NONE: &str = "None";

/// Which heading a path belongs under, and what the row then shows.
///
/// Neither axis is right for every package, which is why this is a control and not
/// a constant. Measured over the operator's eight packages: grouping by `Folder`
/// gives 55 headings for a 717-file package (median group size 3, seventeen of them
/// holding one file), while `Top folder` puts 96% of `proj/quilt-rs-feedback` under
/// a single heading and 73% of `track/2026-09-dpdx` under another. The shape of the
/// package decides which is useful, and only the user can see the shape.
fn group_key(path: &str, mode: &str) -> String {
    match mode {
        GROUP_TOP => path
            .split_once('/')
            .map_or(String::new(), |(head, _)| format!("{head}/")),
        GROUP_NONE => String::new(),
        _ => path
            .rsplit_once('/')
            .map_or(String::new(), |(dir, _)| format!("{dir}/")),
    }
}

/// The file pane, so the whole-page scene composes this code rather than a copy.
#[component]
#[allow(clippy::too_many_lines, reason = "declarative view; length is markup")]
pub fn FilePaneRegion(
    /// Mark the rows a pending decision touches — diverged paths in resolve mode,
    /// incoming paths when the package is behind. This is what `/merge` structurally
    /// cannot do, and it is why `qhq-p85f`'s "and 4 more" line does not need to exist.
    #[prop(optional)]
    marked: bool,
    /// How many rows are ticked. Zero is a state of its own, not a variant of
    /// the same footer — which is what this mock hid until it was asked for.
    #[prop(default = 3)]
    selected: usize,
    /// Unique per instance. `SegmentedControl`'s own doc: "Must be unique on the
    /// page — two controls sharing a name become one group, and selecting in
    /// either clears the other." Several scenes render this region, so a literal
    /// here made every facet bar on the page one radiogroup.
    name: &'static str,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    // `All` excludes ignored files: they are not part of the package, so counting
    // them in "all" makes the number disagree with what a download would ever fetch.
    let facet = RwSignal::new("All 53".to_string());
    let grouping = RwSignal::new(GROUP_FOLDER.to_string());
    let collapsed = RwSignal::new(std::collections::BTreeSet::<String>::new());
    let tick = selected > 0;

    // Rows keyed by heading, in path order. Root-level files come back under the
    // empty key and are rendered first with no heading at all — a "(root)" heading
    // names a directory that does not exist and reads as a real folder.
    let grouped = move || {
        let mode = grouping.get();
        let facet_now = facet.get();
        let wanted = move |status: &str| match facet_now.as_str() {
            "Changed 2" => status == "Changed",
            "Not downloaded 17" => status == "Not downloaded",
            "Ignored 3" => status == "Ignored",
            // `All` is every file that is part of the package, which ignored ones
            // are not — so they appear under exactly one facet, and that facet is
            // what names them.
            _ => status != "Ignored",
        };
        let mut out: Vec<Group> = Vec::new();
        for f in FILES.iter().filter(|f| wanted(f.1)) {
            let key = group_key(f.0, &mode);
            match out.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(f),
                None => out.push((key, vec![f])),
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };

    view! {
        <section class="g-ip-pane g-ip-filepane">
            // Search sits outside any box: it acts on the whole pane, so a border
            // would tie it to whichever band it sat in.
            <div class="g-ip-searchrow">
                <SearchInput value=query aria_label="Search files" placeholder="Search…" />
            </div>

            <div class="g-ip-box g-ip-controls">
                <label class="g-ip-selectall">
                    <input
                        type="checkbox"
                        prop:checked={selected == 17}
                        prop:indeterminate={selected > 0 && selected < 17}
                    />
                    {move || {
                        if selected == 0 {
                            "Select all 17".to_string()
                        } else {
                            format!("{selected} of 17 selected")
                        }
                    }}
                </label>
                <span class="g-ip-spacer" />
                <Select
                    naming=Naming::Prefix("Group".to_string())
                    options=vec![
                        GROUP_FOLDER.to_string(),
                        GROUP_TOP.to_string(),
                        GROUP_NONE.to_string(),
                    ]
                    selected=grouping
                />
                <SegmentedControl
                    aria_label="Filter files"
                    name=name
                    options=vec![
                        "All 53".to_string(),
                        "Changed 2".to_string(),
                        "Not downloaded 17".to_string(),
                        "Ignored 3".to_string(),
                    ]
                    selected=facet
                />
            </div>

            <div class="g-ip-box g-ip-listbox">
            <div class="g-ip-list">
                {move || {
                    grouped()
                        .into_iter()
                        .map(|(key, files)| {
                            let rows = {
                                let key = key.clone();
                                files
                                    .iter()
                                    .map(|f| {
                                        let shown = f.0.strip_prefix(key.as_str()).unwrap_or(f.0);
                                        entry_marked(shown, f.1, f.2, tick, marked && DIVERGED.contains(&f.0))
                                    })
                                    .collect_view()
                                    .into_any()
                            };
                            if key.is_empty() {
                                return rows;
                            }
                            let count = files.len();
                            let k = key.clone();
                            let open = Memo::new(move |_| !collapsed.with(|c| c.contains(&k)));
                            let k2 = key.clone();
                            view! {
                                <div class="g-ip-group">
                                    <div class="g-ip-grouphead">
                                        <button
                                            type="button"
                                            class="g-ip-disclose"
                                            aria-expanded=move || open.get().to_string()
                                            on:click=move |_| {
                                                collapsed
                                                    .update(|c| {
                                                        if !c.remove(&k2) {
                                                            c.insert(k2.clone());
                                                        }
                                                    });
                                            }
                                        >
                                            {move || if open.get() { "▾" } else { "▸" }}
                                        </button>
                                        <input type="checkbox" />
                                        <span class="g-ip-groupname">{key.clone()}</span>
                                        <span class="g-ip-groupcount">{count}</span>
                                    </div>
                                    <div class="g-ip-groupbody" class:g-ip-hidden=move || !open.get()>
                                        {rows}
                                    </div>
                                </div>
                            }
                                .into_any()
                        })
                        .collect_view()
                }}
            </div>

            <Show when=move || { selected > 0 }>
                <footer class="g-ip-panefoot">
                    <span class="g-ip-spacer" />
                    <Button variant=ButtonVariant::Primary on_click=|_| ()>
                        {format!("Download {selected}")}
                    </Button>
                </footer>
            </Show>
            </div>
        </section>
    }
}

// ── Region: the context pane ─────────────────────────────────────────────────

/// The context pane, so the whole-page scene composes this code rather than a copy.
///
/// One mode, always about the package: clicking a row toggles its checkbox and
/// nothing else, so there is no second "focused file" selection to disagree with
/// the first. The three meanings the roadmap asked us to separate land here and in
/// the file pane's footer — standing preference and state display on this side, the
/// one-time action on that one.
#[component]
pub fn ContextPaneRegion(
    /// Unique per instance, for the same reason `FilePaneRegion` takes one: two
    /// panes on the page sharing a radio name become one group, so only one could
    /// ever show a checked scope.
    name: &'static str,
) -> impl IntoView {
    let scope = RwSignal::new("pick");
    view! {
        <aside class="g-ip-pane g-ip-contextpane">
            <div class="g-ip-block">
                <h4>"Revision"</h4>
                <p class="g-ip-revmsg">"\"Add Ernest thread\""</p>
                <p class="g-ip-revwhen">
                    <RelativeTime at=ago(2.0 * HOUR) />
                    " · s3://my-bucket"
                </p>
                // Read-only and named honestly: `list_revisions` returns the revisions
                // this copy holds, dated by when it obtained them. Roadmap #889 is
                // explicit that real published history needs a manifest timestamp and a
                // parent pointer, neither of which exists — so this is not "History".
                <Button on_click=|_| ()>"Revisions you have (4)"</Button>
            </div>

            <div class="g-ip-block">
                <h4>"Keeping"</h4>
                // `checked` as a plain attribute, not `prop:checked`: Leptos writes the
                // property on both radios, and writing `false` to the second clears the
                // group the first just joined. The browser owns exclusivity; the signal
                // only drives the consequence line below.
                <label class="g-ip-choice">
                    <input
                        type="radio"
                        name=name
                        checked=true
                        on:change=move |_| scope.set("pick")
                    />
                    "Files I pick"
                </label>
                <label class="g-ip-choice">
                    <input type="radio" name=name on:change=move |_| scope.set("all") />
                    "The whole package"
                </label>
                <p class="g-ip-consequence">
                    {move || {
                        if scope.get() == "all" {
                            "54 of 56 downloaded — files added later are downloaded too."
                        } else {
                            "54 of 56 downloaded."
                        }
                    }}
                </p>
            </div>
        </aside>
    }
}

/// The context pane in resolve mode, reached by `?resolve=1` so main page v2's
/// queue can navigate straight into it.
#[component]
pub fn ResolvePaneRegion() -> impl IntoView {
    view! {
        <aside class="g-ip-pane g-ip-contextpane">
            <div class="g-ip-block">
                <h4>"Resolve"</h4>
                <p class="g-ip-consequence">
                    "Quilt doesn't merge file contents. Pick which side's revision becomes \
                     the shared one."
                </p>
                // "Yours" / "Published", not "Yours" / "On Quilt": naming the platform
                // as the other place is what principle 4 bans, and `Publish` is already
                // the settled verb, so the published revision is what the other side is.
                //
                // Both sides, named. A binary choice between two revisions that are never
                // shown is the defect this page exists to fix — /merge asks it while
                // rendering neither the revisions nor the files. The messages come from
                // the installed manifest and from `get_revision_message` for the remote,
                // which the version-mismatch banner already fetches today.
                <div class="g-ip-side">
                    <span class="g-ip-sidelabel">"Yours"</span>
                    <span class="g-ip-revmsg">"\"Fix the Ernest thread link\""</span>
                    <span class="g-ip-revwhen">
                        <RelativeTime at=ago(0.4 * HOUR) />
                    </span>
                </div>
                <div class="g-ip-side">
                    <span class="g-ip-sidelabel">"Published"</span>
                    <span class="g-ip-revmsg">"\"Add Caihong folder-upload note\""</span>
                    <span class="g-ip-revwhen">
                        <RelativeTime at=ago(2.0 * HOUR) />
                    </span>
                </div>
                <p class="g-ip-consequence">
                    "2 files differ between these revisions — marked in the list. Replacing \
                     yours overwrites them on this computer."
                </p>
                <Button variant=ButtonVariant::Primary on_click=|_| ()>
                    "Make mine the shared one"
                </Button>
                <Button on_click=|_| ()>"Replace mine with the remote"</Button>
            </div>
        </aside>
    }
}

// ── Scenes ───────────────────────────────────────────────────────────────────

#[component]
fn RegionScenes() -> impl IntoView {
    view! {
        <Scene
            title="Scene · the two panes, side by side"
            note="The file pane grows, the context pane is fixed at roughly 280px. \
                  \
                  One action, and it can only mean one thing: the rows that are ticked. \
                  There is no standing 'download everything' button — wanting everything is \
                  already two other things, ticking all and downloading, or setting Keeping \
                  to the whole package, and a third path would compete with both. With a \
                  single action left, no control has to name an extent. \
                  \
                  Only a row that can be downloaded carries a checkbox. A tick on a file \
                  already here would have nothing to act on, and it is what made a \
                  'select all 56' disagree with a 'download 17'."
        >
            <div class="g-ip-panes">
                <FilePaneRegion name="ip-a" />
                <ContextPaneRegion name="ip-a-scope" />
            </div>
        </Scene>

        <Scene
            title="Scene · nothing selected"
            note="The same pane with no rows ticked, which is how it opens. The whole \
                  footer is absent — not a disabled button, not an empty bar — because \
                  there is nothing to do with an empty selection, and the select-all \
                  checkbox in the toolbar is where a selection begins. \
                  \
                  The checkbox is tri-state: unchecked here, indeterminate at 3 of 17, \
                  checked at 17. A button could not have expressed the way back."
        >
            <div class="g-ip-panes">
                <FilePaneRegion selected=0 name="ip-b" />
                <ContextPaneRegion name="ip-b-scope" />
            </div>
        </Scene>

        <Scene
            title="Scene · resolve is a page mode, not a dialog"
            note="Reached by ?resolve=1. The choice is in the pane and the diverged files \
                  are marked in the list, so the user can scroll, search and open a file \
                  before choosing. A modal would cover the list at the exact moment of an \
                  irreversible decision — which is qhq-8mgw.65, open and P2, one level down. \
                  The page never puts a CHOICE in a modal; only a confirmation of something \
                  irreversible, which is what 'Replace mine with the remote' gets next."
        >
            <div class="g-ip-panes">
                <FilePaneRegion marked=true name="ip-c" />
                <ResolvePaneRegion />
            </div>
        </Scene>

        <Scene
            title="Scene · narrow, below ~800px"
            note="Context above files. The region with no upper bound on length goes last, \
                  which would be the right stack even if it were chosen for looks. minWidth \
                  is 1024, so the app cannot reach this by dragging — but minWidth is a \
                  request to the window manager, not a guarantee: a 1366×768 screen at 150% \
                  scaling has roughly 911 logical pixels. One media query over content \
                  already kept single-column."
        >
            <div class="g-ip-panes g-ip-panes--narrow">
                <ContextPaneRegion name="ip-d-scope" />
                <FilePaneRegion name="ip-d" />
            </div>
        </Scene>
    }
}

#[component]
fn WholePage() -> impl IntoView {
    view! {
        <Scene
            title="Scene · the whole page, at the height floor"
            note="The frame is 560px — the window's minimum — so the fold is the real one. \
                  This is the scene that tests the design's central bet. v1 stacks six bands \
                  (appbar, toolbar, status banner, scope band, entries toolbar, action bar) \
                  and leaves roughly 240px for a list that may hold 717 rows; count what \
                  survives here instead. \
                  \
                  Note what is absent: no bottom action bar, because the primary action is in \
                  the header — which is Christian's 2026-06-25 complaint answered structurally \
                  rather than by relabelling a button."
        >
            <div class="g-window">
                <PageLayout actions=appbar_actions()>
                    <PackageHeaderRegion
                        state_label="Newer revision available"
                        tone=StateTone::Attention
                        action=Some("Get latest".to_string())
                    />
                    <div class="g-ip-panes">
                        <FilePaneRegion marked=true name="ip-e" />
                        <ContextPaneRegion name="ip-e-scope" />
                    </div>
                </PageLayout>
            </div>
        </Scene>
    }
}

#[component]
fn NothingToSay() -> impl IntoView {
    let dismissed = RwSignal::new(false);
    view! {
        <Scene
            title="Scene · when the page has little to show"
            note="Three states that are easy to leave until last and are then wrong. The \
                  paused banner is an annotation, not a state — the package still has a real \
                  upstream state underneath, so the pause rides in the layout's banner slot \
                  rather than replacing the header's label, and it cannot render without a \
                  reason because the reason is its content (qhq-8mgw.60). The blankslate \
                  teaches rather than reports. The truncation line exists because the backend \
                  stops building entries at 1000 and nothing has ever said so."
        >
            <Cell label="autosync paused — the reason IS the content" full=true>
                <Show when=move || !dismissed.get()>
                    <Banner
                        variant=BannerVariant::Warning
                        on_dismiss=move |_| dismissed.set(true)
                    >
                        "Automatic syncing is paused: sign-in expired for demo.quiltdata.com. \
                         It resumes when you publish or get the latest revision."
                    </Banner>
                </Show>
            </Cell>
            <Cell label="a package with no files yet" wide=true>
                <Blankslate
                    heading="No files yet"
                    description="Add files to this package's folder, then publish them."
                    primary_action=view! {
                        <Button on_click=|_| ()>"Open folder"</Button>
                    }
                        .into_any()
                />
            </Cell>
            <Cell label="over the 1000-entry cap" wide=true>
                <p class="g-ip-consequence">
                    "This package has 4,312 files. Showing the first 1,000."
                </p>
            </Cell>
        </Scene>
    }
}
