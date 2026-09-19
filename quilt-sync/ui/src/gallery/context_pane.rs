//! The installed-package page's context pane, in the states it can hold.
//!
//! # What this scene is for
//!
//! The pane is the page's *other* half, and every one of its parts already has a
//! Core cell. What no Core cell can show is the three of them at 280px with a
//! popover hanging off one — whether the surface fits the pane it belongs to,
//! whether two blocks labelled by two different components look like siblings,
//! and whether the conditional download action reads as part of the choice above
//! it or as a control that wandered in.
//!
//! # The pane is a `Card` inside an `<aside>`
//!
//! `Card` is a `<section>` with an `<h2>`, and this pane has no visible title and
//! is not a section of the page's prose — so the landmark and the name live on
//! the `<aside>`, and the `Card` inside it is a box drawing a border. One
//! element is spent to say what the pane is; the alternative was a level prop on
//! `Card`, which would have put the pane's structure into every other caller's
//! signature.
//!
//! # Two blocks, two components, one label style
//!
//! `Revision` is a [`PaneSection`](crate::kit::PaneSection) label and `Keeping`
//! is a [`ChoiceGroup`](crate::kit::ChoiceGroup) label, because a `PaneSection`
//! titled `Keeping` around a `ChoiceGroup` titled `Keeping` says it twice. They
//! are drawn from the same rule and match to the pixel — but only one of them is
//! a heading, so heading navigation finds `Revision` and walks straight past
//! `Keeping`. That is the pane's shape reporting a kit gap, not a scene's
//! mistake.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::BackLink;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::CatalogLink;
use crate::kit::Choice;
use crate::kit::ChoiceGroup;
use crate::kit::LoadFailure;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::SkeletonBox;

/// The design's fixed pane width. Written here rather than taken from a token
/// because it is this page's bet, not the system's: the file list grows and the
/// pane does not.
const PANE: &str = "width:280px";

const NAMESPACE: &str = "user/plate-07";
/// A fact about the package, so it sits in the section and never on a row —
/// inside the revisions surface it would repeat identically all the way down.
const BUCKET: &str = "s3://quilt-lab-plates";

const TOTAL: usize = 56;

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// The revisions this copy holds, newest first — which is what `list_revisions`
/// returns and what the trigger counts. Four, because the trigger is hidden at
/// one and a list of two never shows whether the surface scrolls.
///
/// The newest has not been sent: that is the ordinary shape of this list, since
/// the revision somebody is working on is the one they have not published, and a
/// list where every row is identical on the one axis the glyph draws would prove
/// nothing about the glyph.
fn revisions() -> Vec<(&'static str, f64, Option<CatalogLink>)> {
    vec![
        ("Add Caihong folder-upload note", ago(2.0 * HOUR), None),
        (
            "Re-run plate 7 with the corrected layout",
            ago(3.0 * DAY),
            Some(catalog("c41d8f")),
        ),
        ("", ago(9.0 * DAY), Some(catalog("9a2b71"))),
        ("Initial upload", ago(26.0 * DAY), Some(catalog("06e3ad"))),
    ]
}

/// Where a published revision is read. The hash is banned from the page's words
/// and belongs in an address, which is not a name.
///
/// The opener does nothing here. In the app it is `open_in_web_browser`; a
/// gallery has no Tauri host to hand an address to, and letting the anchor
/// follow itself would take the gallery with it.
fn catalog(revision: &str) -> CatalogLink {
    CatalogLink::new(
        format!(
            "https://quilt-lab.example/b/quilt-lab-plates/packages/{NAMESPACE}/tree/{revision}/"
        ),
        Callback::new(|_url: String| ()),
    )
}

fn scopes() -> Vec<Choice> {
    vec![
        Choice::new("pick", "Files I pick"),
        Choice::new("all", "The whole package"),
    ]
}

/// The trigger owes `aria-expanded` and `aria-controls`; the overlay hands the id
/// over so it can point at what it opens.
fn trigger(open: RwSignal<bool>) -> impl FnOnce(String) -> AnyView {
    move |surface_id| {
        view! {
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                "Revisions you have (4)"
            </Button>
        }
        .into_any()
    }
}

/// The surface, filled.
fn revision_list() -> AnyView {
    view! {
        <PaneSection>
            {revisions()
                .into_iter()
                .map(|(message, at, catalog)| {
                    view! {
                        <RevisionRow
                            message=message
                            at=at
                            published=catalog.is_some()
                            catalog=catalog
                        />
                    }
                })
                .collect_view()}
        </PaneSection>
    }
    .into_any()
}

/// The surface before `list_revisions` answers. Four bars for four rows, at the
/// widths messages actually have, so the surface does not resize under the
/// reader's cursor when it fills.
fn revision_skeleton() -> AnyView {
    view! {
        <PaneSection>
            <SkeletonBox width="200px" />
            <SkeletonBox width="228px" />
            <SkeletonBox width="164px" />
            <SkeletonBox width="188px" />
        </PaneSection>
    }
    .into_any()
}

/// What the choice means, in the present tense.
///
/// Two clauses from v1's own band, kept apart on purpose: the count reports the
/// present, and the standing line promises something about files that do not
/// exist yet. Only whole-package scope may make that promise — under
/// individual-file scope the next revision can add a file and falsify it.
fn consequence(scope: RwSignal<String>, pending: usize) -> Signal<String> {
    Signal::derive(move || {
        let counted = if pending == 0 {
            "All files are downloaded".to_string()
        } else {
            format!("{} of {TOTAL} downloaded", TOTAL - pending)
        };
        if scope.get() == "all" {
            format!("{counted} — files added later are downloaded too.")
        } else {
            format!("{counted}.")
        }
    })
}

/// The backlog action. Conditional and counted, which is what lets it exist at
/// all: a standing `Download all files` with no extent was removed on purpose,
/// and this appears only where the scope change would otherwise strand files
/// already listed.
fn download(scope: RwSignal<String>, pending: usize) -> AnyView {
    view! {
        {move || {
            (scope.get() == "all" && pending > 0)
                .then(|| {
                    let label = if pending == 1 {
                        "Download 1 file".to_string()
                    } else {
                        format!("Download {pending} files")
                    };
                    view! { <Button on_click=|_| ()>{label}</Button> }
                })
        }}
    }
    .into_any()
}

/// The pane in its default mode.
///
/// The two blocks are direct children of the `Card`, which is what draws the rule
/// between them; `PaneSection` supplies the air on either side of it. A flush
/// hairline is right for rows, which carry their own padding — against a section
/// that has none it lands on the last control and reads as its underline.
fn pane(
    open: RwSignal<bool>,
    body: AnyView,
    scope: RwSignal<String>,
    pending: usize,
    on_page: bool,
) -> AnyView {
    let sections = view! {
        <PaneSection label="Revision">
            <RevisionRow message="Add Caihong folder-upload note" at=ago(2.0 * HOUR) />
            <span style="color:var(--q-fgColor-muted); font-size:var(--q-text-body)">
                {BUCKET}
            </span>
            <AnchoredOverlay
                trigger=trigger(open)
                open=open
                aria_label="Revisions you have"
                align=Align::End
            >
                {body}
            </AnchoredOverlay>
        </PaneSection>
        <PaneSection>
            <ChoiceGroup
                label="Keeping"
                caption=consequence(scope, pending)
                options=scopes()
                selected=scope
            />
            {download(scope, pending)}
        </PaneSection>
    };

    // On the page the width is a class, not an inline style: stacked under 800px
    // the pane stops sharing a row with anything and takes the column, and a
    // container query cannot outrank an attribute.
    view! {
        <aside
            aria-label="About this package"
            class=on_page.then_some("g-ip-contextpane")
            style=(!on_page).then_some(PANE)
        >
            <Card>{sections}</Card>
        </aside>
    }
    .into_any()
}

/// The pane where the page puts it: last in a row, with the file list holding
/// everything to its left.
///
/// The three overlay cells are drawn inside this and the others are not, because
/// the surface is the one part of the pane whose behaviour is about the pane's
/// *position*. A 280px pane floating alone in a cell would let a popover open
/// rightwards into empty gallery, which is the one direction the real page does
/// not have.
fn in_page(pane: AnyView) -> AnyView {
    view! {
        <div style="display:flex; gap:var(--q-space-3); width:992px; max-width:100%">
            <div style="flex:1; min-width:0; display:flex; align-items:center; \
                        justify-content:center; min-height:180px; \
                        border:1px dashed var(--q-borderColor-muted); \
                        border-radius:var(--q-radius); color:var(--q-fgColor-muted); \
                        font-size:var(--q-text-body)">
                "the file list"
            </div>
            {pane}
        </div>
    }
    .into_any()
}

/// The pane under `?resolve=1`.
///
/// No outer heading: the mode is already named by the sentence under it, and a
/// `PaneSection("Resolve")` wrapping two labelled sides draws the same label
/// style at two levels, which is a hierarchy the reader cannot see.
///
/// The two buttons are deliberately not mirrors. One publishes what is already
/// here; the other replaces local files and is the one that opens a `Dialog`. The
/// kit has no Danger button and should not — Danger is a *status* colour in this
/// system, so a red confirm would read as *this errored* — which leaves the
/// weight to be carried by the arrangement and by the dialog's own copy.
fn resolve() -> AnyView {
    view! {
        <aside aria-label="About this package" style=PANE>
            <Card>
                <div class="g-stack" style="gap:var(--q-space-3)">
                    <BackLink href="#contextpane" label=NAMESPACE />
                    <PaneSection>
                        <p style="margin:0">
                            "2 files differ between these revisions — marked in the list."
                        </p>
                        <PaneSection nested=true label="Yours">
                            <RevisionRow
                                message="Re-run plate 7 with the corrected layout"
                                at=ago(0.4 * HOUR)
                            />
                        </PaneSection>
                        <PaneSection nested=true label="Published">
                            <RevisionRow
                                message="Add Caihong folder-upload note"
                                at=ago(2.0 * HOUR)
                            />
                        </PaneSection>
                        <div class="g-stack" style="gap:var(--q-space-2)">
                            <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                "Make mine the shared one"
                            </Button>
                            <Button on_click=|_| ()>"Replace mine with the published one"</Button>
                        </div>
                    </PaneSection>
                </div>
            </Card>
        </aside>
    }
    .into_any()
}

/// What the cells are for, and what looking at them settled.
const NOTE: &str = "280px holding two blocks: what the page says about the package rather \
    than about its files. \
    \
    Flip a radio and the caption and the download action answer together. \
    Click a trigger: the surface hangs leftwards over the file list, which is \
    the only direction the page has for it. In the list a published revision \
    wears a cloud and links to the catalog; the unsent one wears the slashed \
    cloud. \
    \
    Unresolved: `Replace mine with the published one` does not fit 280px and \
    truncates.";

/// The region itself, for the whole-page scene.
///
/// The revisions surface opens leftwards over the file list, which is the one
/// part of this pane whose behaviour is about where the pane sits — so on the
/// page it is drawn by the real arrangement rather than by a cell imitating it.
#[component]
pub fn ContextPaneRegion(
    /// `?resolve=1`: the pane swaps to the choice between two revisions.
    #[prop(optional)]
    resolving: bool,
    /// The standing scope, shared with whatever else on the page reads it.
    scope: RwSignal<String>,
    /// How many files the scope leaves outstanding, which is what decides
    /// whether `Keeping` carries a download action at all.
    #[prop(optional)]
    pending: usize,
) -> impl IntoView {
    let open = RwSignal::new(false);
    if resolving {
        resolve()
    } else {
        pane(open, revision_list(), scope, pending, true)
    }
}

#[component]
pub fn ContextPaneScene() -> impl IntoView {
    // One open signal per pane: a popover of `auto` type closes any other, so
    // sharing one between two cells would make the second trigger reopen the
    // first cell's surface.
    let resting = RwSignal::new(false);
    let outstanding = RwSignal::new(false);
    let settled = RwSignal::new(false);
    let listed = RwSignal::new(false);
    let waiting_surface = RwSignal::new(false);
    let failed = RwSignal::new(false);

    let pick = RwSignal::new("pick".to_string());
    let whole = RwSignal::new("all".to_string());
    let complete = RwSignal::new("all".to_string());
    let listing = RwSignal::new("pick".to_string());
    let waiting = RwSignal::new("pick".to_string());
    let broken = RwSignal::new("pick".to_string());

    view! {
        <Scene
            title="The context pane"
            note=NOTE
        >
            <Cell wide=true label="at rest — files I pick, two outstanding">
                {pane(resting, revision_list(), pick, 2, false)}
            </Cell>
            <Cell wide=true label="the whole package, two files outstanding">
                {pane(outstanding, revision_list(), whole, 2, false)}
            </Cell>
            <Cell wide=true label="the whole package, nothing outstanding — no action">
                {pane(settled, revision_list(), complete, 0, false)}
            </Cell>
            <Cell full=true label="click the trigger — the revisions this copy holds">
                {in_page(pane(listed, revision_list(), listing, 2, false))}
            </Cell>
            <Cell full=true label="click it — the call has not answered yet">
                {in_page(pane(waiting_surface, revision_skeleton(), waiting, 2, false))}
            </Cell>
            <Cell full=true label="click it — the call failed">
                {in_page(
                    pane(
                        failed,
                        view! {
                            <LoadFailure
                                words="Could not load your revisions."
                                on_retry=Callback::new(|()| ())
                            />
                        }
                            .into_any(),
                        broken,
                        2,
                        false,
                    ),
                )}
            </Cell>
            <Cell wide=true label="resolve mode, with the exit the design left open">
                {resolve()}
            </Cell>
        </Scene>
    }
}
