//! Combined · the file pane's list toolbar.
//!
//! `SelectAll` + `Select` + `SegmentedControl` inside a `ListToolbar`, with
//! `SearchInput` on its own row above. Both rows sit bare — verdict 17, they act
//! on the list rather than being part of it — and neither component is new.
//!
//! This section exists to answer two questions by looking, which is why it is
//! framed at a width rather than left to fill the gallery.
//!
//! # It wraps at 1024. Measured, not argued.
//!
//! `ListToolbar` is `flex-wrap: wrap` with no breakpoint, deliberately: a narrow
//! window stacks the controls instead of scrolling them out of reach. The
//! installed-package design then claims the toolbar is *two rows at every
//! width*, which is only true if the wrap never fires.
//!
//! It fires. The three controls and their gaps are **680px** wide, the inset
//! leaves the toolbar the pane minus 28, and the narrowest pane that holds one
//! line is therefore **708px**. The pane at 1024 is 700: `1024 − 32` of
//! `page_layout`'s own `padding-inline` leaves 992, less the context pane's
//! fixed 280 and a `space-3` gap. Eight pixels short, and twelve if the shell
//! takes `space-4` — before any scrollbar.
//!
//! So the toolbar is **three rows at 1024**, not two, and the row it gains is
//! 40px of the vertical budget that was counted as saved. The first two cells
//! are the pane at 1024 and the narrow arrangement; the third is 708px, the
//! width where it just fits, because a claim that turns over eight pixels
//! should be readable as eight pixels.
//!
//! **The facets are what spends it.** Of the 680, the segmented control is 396
//! — 58% of the toolbar for four options, none of them truncated. Nothing here
//! is broken; it simply does not fit, and the facets are where the width went.
//!
//! The frame is a proxy and not the page: 700px of gallery is not a 1024
//! viewport with a 280px context pane beside it. It is the same content at the
//! same width, which is what settles whether the controls fit; the pane's own
//! margins are the shell's to confirm when it is built.
//!
//! # Four counted facets, against the control's own guidance
//!
//! `SegmentedControl`'s module doc says *two or three short options here, more
//! than that in a `Select`*. The facets are four, and counted: `All 53 ·
//! Changed 2 · Not downloaded 17 · Ignored 3`. Verdict 13 used it anyway and
//! recorded no override.
//!
//! At this width the rule of thumb turns out to be about width and not taste:
//! the four segments are 396px, the toolbar needs 708 and has 700, and the same
//! four choices in a `Select` would read `Show: Not downloaded 17` in about 190
//! — which is the row back, with 200px to spare. That is the trade, on screen,
//! and it is a design decision rather than a defect: all four counts visible at
//! a glance, against a toolbar that fits.
//!
//! # Every number here is counted, not asserted
//!
//! The facet counts, `SelectAll`'s total and what the search narrows to all come
//! from one fixture, so they cannot drift from each other — the pane's *one
//! question across three counters*, answered once. Nothing renders the rows:
//! they exist to be counted, because a toolbar's numbers are the only thing it
//! says.
//!
//! The counts are over the whole fixture and do not move when the search does —
//! which is also what keeps this control correct. `SegmentedControl` selects by
//! the option's own string, so a label carrying a count that changed under the
//! user would leave the selection matching no option at all.
//!
//! # Left, then right — and upwards when it stacks
//!
//! `reverse_when_stacked` puts select-all on the **last** line, against the rows
//! it acts on, and the facets on the first. Only the container knows there is a
//! wrap to order, which is why the prop is `ListToolbar`'s and not an
//! arrangement the caller can express by ordering its children: order and wrap
//! order are the same thing in flexbox, so a caller reordering for the stacked
//! case would reorder the single-line case too.
//!
//! # Left, then right
//!
//! Select-all sits left, on the rows' own checkbox column. The free space is
//! immediately after it, so grouping and the facets travel together against the
//! right edge and the facets finish **flush with the search field above them** —
//! the two rows share one right margin, which is the only thing that makes them
//! read as one block of controls rather than two unrelated rows.
//!
//! # The left inset is part of the measurement
//!
//! Select-all sits in the rows' checkbox column, which puts the toolbar's
//! content 28px in from the pane's edge — the list box's own `space-3` plus the
//! 16px disclosure gutter. There are no rows here to align to, so the inset
//! draws nothing; it is here because leaving it out measures a toolbar 28px
//! wider than the one that ships, and 28px is the whole margin this question is
//! about. The alignment itself is the file list's section.
//!
//! The search row above keeps the pane's full width — §4 has it alone and full
//! width, so it starts where the pane starts and not where the boxes do.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ListToolbar;
use crate::kit::Naming;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::kit::SelectAll;

/// What the pane knows about a file, reduced to what the toolbar counts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Here and unchanged. Most of the list.
    Here,
    /// Here, and different from the revision.
    Changed,
    /// Not here — the only mark a tick can act on.
    Missing,
    /// Excluded from `All` by verdict 21, and shown by exactly one facet.
    Ignored,
}

/// The package, as a list of marks and names. Sized to the design's own numbers
/// so the labels on screen are the labels under discussion: 56 files, of which 3
/// are ignored, leaving `All 53` — and `Changed 2`, `Not downloaded 17`.
fn fixture() -> Vec<(String, Mark)> {
    let mut files = vec![
        ("README.md".to_string(), Mark::Here),
        ("quilt_summarize.json".to_string(), Mark::Here),
        ("manifest.jsonl".to_string(), Mark::Missing),
        ("notes/ernest-thread.md".to_string(), Mark::Changed),
        ("notes/caihong-upload.md".to_string(), Mark::Changed),
        (".DS_Store".to_string(), Mark::Ignored),
        ("notes/.DS_Store".to_string(), Mark::Ignored),
        ("raw/.DS_Store".to_string(), Mark::Ignored),
    ];
    for i in 1..=6 {
        files.push((format!("notes/handoff-{i:02}.md"), Mark::Here));
    }
    for i in 1..=3 {
        files.push((
            format!("investigations/2026-09-15-installed-package-page/design-{i:02}.md"),
            Mark::Here,
        ));
    }
    for i in 4..=5 {
        files.push((
            format!("investigations/2026-09-15-installed-package-page/verdicts-{i:02}.md"),
            Mark::Missing,
        ));
    }
    for i in 1..=23 {
        files.push((format!("raw/plate-{i:02}.csv"), Mark::Here));
    }
    for i in 24..=37 {
        files.push((format!("raw/plate-{i:02}.csv"), Mark::Missing));
    }
    files
}

/// A facet: the word it shows, and what it admits.
struct Facet {
    word: &'static str,
    admits: fn(Mark) -> bool,
}

/// The four. `All` excludes ignored files — verdict 21, and the reason the
/// counts are 53 and not 56.
const FACETS: [Facet; 4] = [
    Facet {
        word: "All",
        admits: |m| !matches!(m, Mark::Ignored),
    },
    Facet {
        word: "Changed",
        admits: |m| matches!(m, Mark::Changed),
    },
    Facet {
        word: "Not downloaded",
        admits: |m| matches!(m, Mark::Missing),
    },
    Facet {
        word: "Ignored",
        admits: |m| matches!(m, Mark::Ignored),
    },
];

/// `All 53`, and so on. Counted from the fixture, over the whole package rather
/// than the current view: the toolbar still has to render when the list is
/// narrowed, and a count that moved under the reader would break the control
/// that carries it.
fn labels() -> Vec<String> {
    let files = fixture();
    FACETS
        .iter()
        .map(|facet| {
            let n = files.iter().filter(|(_, m)| (facet.admits)(*m)).count();
            let word = facet.word;
            format!("{word} {n}")
        })
        .collect()
}

/// The two control rows at one width.
///
/// `width` is the frame, not a breakpoint the toolbar knows about: it has none,
/// which is exactly the thing being measured.
fn toolbar(width: &'static str) -> AnyView {
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("Base folder".to_string());
    let options = labels();
    let facet = RwSignal::new(options[0].clone());
    let picked = RwSignal::new(0_usize);

    // What the current facet and the current search leave on screen, and how
    // many of those a tick can act on — only a missing file can be downloaded,
    // so the two numbers part company under every facet but one.
    let shown = Signal::derive(move || {
        let (f, q) = (facet.get(), query.get().to_lowercase());
        let admits = FACETS
            .iter()
            .find(|facet| f.starts_with(facet.word))
            .map_or(FACETS[0].admits, |facet| facet.admits);
        fixture()
            .into_iter()
            .filter(|(name, m)| admits(*m) && name.to_lowercase().contains(&q))
            .filter(|(_, m)| *m == Mark::Missing)
            .count()
    });

    // The label says `shown` whenever a control has narrowed the view — a facet
    // can narrow to exactly the number of rows the package has, so the count
    // alone cannot say it and the caller must.
    let narrowed =
        Signal::derive(move || !query.get().is_empty() || !facet.get().starts_with("All"));

    // Reset rather than clamp: the tick is over rows, and a facet change is a
    // different set of rows. Carrying a number across would claim selections
    // that are no longer on screen.
    Effect::new(move |_| {
        let _ = (facet.get(), query.get());
        picked.set(0);
    });

    view! {
        <div class="g-stack" style=format!("width:{width}; max-width:100%; gap:var(--q-space-2)")>
            // Alone and full width. The search acts on the list, so it sits
            // above the controls rather than among them.
            <div style="display:flex">
                <SearchInput
                    value=query
                    aria_label="Search files in this package"
                    placeholder="Search files…"
                />
            </div>
            // 28px: the list box's own `space-3` plus the 16px disclosure
            // gutter, which is where the rows put their boxes and therefore
            // where select-all has to start. It draws nothing here and costs
            // width, which is the point — the toolbar has 28px less than the
            // pane to fit five controls into.
            <div style="padding-left:calc(var(--q-space-3) + 16px)">
            <ListToolbar reverse_when_stacked=true>
                // A select-all over nothing is a control that cannot act — the
                // `Changed` facet reaches that, because a file that is here
                // cannot be downloaded.
                <Show when=move || { shown.get() > 0 }>
                    <SelectAll
                        selected=picked
                        total=shown
                        narrowed=narrowed
                        on_toggle=move |next| picked.set(if next { shown.get() } else { 0 })
                    />
                </Show>
                // Grouping and the facets as one right-hand group, so the free
                // space lands after select-all and the facets finish flush with
                // the search field above. It wraps within itself and keeps
                // justifying right, which is what holds the shared right margin
                // at the widths where the row does not fit on one line — and at
                // 1024 it does not.
                <div style="margin-left:auto; display:flex; gap:var(--q-space-2); \
                            flex-wrap:wrap-reverse; justify-content:flex-end">
                    <Select
                        naming=Naming::Prefix("Group".to_string())
                        options=vec!["Base folder".to_string(), "None".to_string()]
                        selected=group
                    />
                <SegmentedControl
                    aria_label="Filter files"
                    // One name per cell. `SegmentedControl` takes the radio
                    // group's name from its caller, and two instances sharing
                    // one make a single group across both — the bug the kit
                    // already logged when five facet bars became one.
                    name=match width {
                        "700px" => "facets-pane",
                        "708px" => "facets-fits",
                        _ => "facets-narrow",
                    }
                    options=options
                    selected=facet
                />
                </div>
            </ListToolbar>
            </div>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The file pane's two control rows — and at 1024 they are three. The \
                    controls and their gaps are 680px and the pane leaves the toolbar 700, \
                    so one line needs 708: it wraps by eight pixels, and the row it gains is \
                    40px nobody budgeted. The three cells are that width, the width where it \
                    just fits, and the narrow arrangement. The facets are where the width \
                    goes: 396 of the 680, 58% of the toolbar, for four options against \
                    SegmentedControl's own two or three. Nothing truncates; it simply does \
                    not fit. Type in the search and the label says `shown`; choose `Changed` \
                    and select-all disappears.";

#[component]
pub fn FileToolbarStories() -> impl IntoView {
    view! {
        <Story title="The list toolbar" note=NOTE>
            <Cell full=true label="700px — the file pane at 1024. Two lines: it wraps">
                {toolbar("700px")}
            </Cell>
            <Cell full=true label="708px — the narrowest pane that holds one line">
                {toolbar("708px")}
            </Cell>
            <Cell full=true label="460px — the narrow arrangement, where wrapping is the point">
                {toolbar("460px")}
            </Cell>
        </Story>
    }
}
