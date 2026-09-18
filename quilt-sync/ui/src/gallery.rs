//! Component gallery — a debug harness, never shipped and never linked to.
//!
//! Its own Trunk target (`gallery.html`), mounting `Gallery` and never `App`. A
//! deliberate separation, not just a style choice: `App` mounts the real router
//! and app-level effects such as `UpdateChecker`'s on-mount version check, none
//! of which belong in a harness whose whole point is rendering component
//! stories in isolation, outside any page's own data-fetching and navigation.
//! (`tauri::invoke`/`invoke_unit` now return `Err` rather than trapping when
//! `window.__TAURI__` is missing, so `UpdateChecker`'s own on-mount call is no
//! longer a crash risk here — but `tauri_listen_raw` is still declared without
//! `catch`, so a page reachable through `App`'s router that calls `listen()`
//! (`installed_packages_list.rs`, say) would still trap the module outside
//! Tauri. The isolation stays correct regardless.)
//!
//! Run it with `just gallery` and iterate in Chrome or Firefox for
//! speed — then check **GNOME Web (Epiphany)**, which is `WebKitGTK`, before
//! committing a layout. Chrome is not the webview that ships on Linux.
//!
//! # Render states, not components
//!
//! One entry per *state*. A gallery listing `Button` once is useless; listing
//! its twelve is a visual-regression surface. Add a cell for every state a
//! component can reach, including the ones that only appear when something has
//! gone wrong.

// `main.rs`'s line: this bin is its own compilation unit, so `configure!`
// there does not reach tests compiled here. `kit`'s DOM tests (`file_row.rs`)
// run in this binary too — without this, `web_sys::window()` is `None` under
// wasm-bindgen-test's default Node.js target.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

// A plain `mod`, no `#[path]`. This file lives in `src/` next to `kit.rs`
// precisely so that works: a `#[path]`-included module resolves its *children*
// relative to itself, so `kit.rs`'s `pub mod button;` would have looked for
// `src/button.rs` and failed. Declared as a second `[[bin]]` in Cargo.toml.
// From the library beside this file, not a second compilation of the same tree —
// see `src/lib.rs`. `pub(crate)` so the story modules keep reaching it as
// `crate::kit`, which is what they were written against.
pub(crate) use quilt_sync_ui::kit;

// One module per component. Adding a story means adding a file here and one line
// in `Gallery` below — there is no registry to keep in step.
mod gallery {
    pub mod action_menu;
    pub mod anchored_overlay;
    pub mod back_link;
    pub mod button;
    pub mod card;
    pub mod checkbox;
    pub mod choice_group;
    pub mod countdown;
    pub mod entry_group;
    pub mod entry_row;
    pub mod feedback;
    pub mod file_list;
    pub mod file_toolbar;
    pub mod forms;
    pub mod host_row;
    pub mod list_toolbar;
    pub mod load_failure;
    pub mod package_header;
    pub mod packages;
    pub mod page;
    pub mod pane_section;
    pub mod queue;
    pub mod recent_files;
    pub mod revision_row;
    pub mod search_input;
    pub mod segmented_control;
    pub mod select;
    pub mod select_all;
    pub mod skeleton;
    pub mod split_button;
    pub mod state_label;
    pub mod state_strip;
    pub mod toggle_row;
    pub mod unchecked;
}

use leptos::prelude::*;

use crate::gallery::action_menu::ActionMenuStories;
use crate::gallery::anchored_overlay::AnchoredOverlayStories;
use crate::gallery::back_link::BackLinkStories;
use crate::gallery::button::ButtonStories;
use crate::gallery::card::CardStories;
use crate::gallery::checkbox::CheckboxStories;
use crate::gallery::choice_group::ChoiceGroupStories;
use crate::gallery::countdown::CountdownStories;
use crate::gallery::entry_group::EntryGroupStories;
use crate::gallery::entry_row::EntryRowStories;
use crate::gallery::feedback::BannerScene;
use crate::gallery::feedback::FeedbackStories;
use crate::gallery::file_list::FileListStories;
use crate::gallery::file_toolbar::FileToolbarStories;
use crate::gallery::forms::DialogScene;
use crate::gallery::forms::FormsStories;
use crate::gallery::host_row::HostRowStories;
use crate::gallery::list_toolbar::ListToolbarScene;
use crate::gallery::load_failure::LoadFailureStories;
use crate::gallery::package_header::PackageHeaderScene;
use crate::gallery::packages::PackageRowStories;
use crate::gallery::packages::PackagesScene;
use crate::gallery::page::PageScene;
use crate::gallery::pane_section::PaneSectionStories;
use crate::gallery::queue::QueueScene;
use crate::gallery::queue::QueueStories;
use crate::gallery::recent_files::RecentFilesScene;
use crate::gallery::recent_files::RecentFilesStories;
use crate::gallery::revision_row::RevisionRowStories;
use crate::gallery::search_input::SearchInputStories;
use crate::gallery::segmented_control::SegmentedControlStories;
use crate::gallery::select::SelectStories;
use crate::gallery::select_all::SelectAllStories;
use crate::gallery::skeleton::SkeletonStories;
use crate::gallery::split_button::SplitButtonStories;
use crate::gallery::state_label::StateLabelStories;
use crate::gallery::state_strip::PausedScene;
use crate::gallery::state_strip::StateStripScene;
use crate::gallery::state_strip::StripErrorScene;
use crate::gallery::toggle_row::ToggleRowStories;
use crate::gallery::unchecked::UncheckedScene;
use kit::Button;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(Gallery);
}

#[component]
fn Gallery() -> impl IntoView {
    let dark = RwSignal::new(false);

    // The app's own switch, shared: the gallery drives it from a button and
    // the app from the OS, and one of them getting the root element wrong is
    // exactly the bug the shared version's doc explains.
    Effect::new(move |_| quilt_sync_ui::theme::set(dark.get()));
    // A v2 reader, so v2-only rules apply — the loading frame's ground is one.
    quilt_sync_ui::theme::set_v2(true);

    // One list, used twice: the index reads the labels, the page consumes the
    // views. A second hardcoded list of section names would drift from this one
    // the first time anybody added a component.
    //
    // Anchors rather than tabs. Tabs would show one component at a time, which
    // costs the thing a design-system gallery is *for*: noticing that a Select is
    // a pixel taller than a Button, or that two components disagree about a
    // baseline. The scenes exist precisely to compare, and Ctrl+F stops working
    // across hidden panels. So the page stays one scroll and gains a way to jump.
    // One list, used twice: the index reads the labels, the page consumes the
    // views. A second hardcoded list of section names would drift from this one
    // the first time anybody added a component.
    //
    // Anchors rather than tabs. Tabs would show one component at a time, which
    // costs the thing a design-system gallery is *for*: noticing that a Select is
    // a pixel taller than a Button, or that two components disagree about a
    // baseline. The scenes exist precisely to compare, and Ctrl+F stops working
    // across hidden panels. So the page stays one scroll and gains a way to jump.
    //
    // The four tiers are what a section IS, not where it sits in a menu:
    //
    //   Core      one kit component, every state it can reach, no composition.
    //   Combined  several kit components wired as the kit intends, no page data.
    //   Scenes    one page REGION in one state, with fixture data.
    //   Pages     a whole page at a real viewport.
    //
    // The tier is the heading, so a section's own label does not repeat it —
    // `Scene · list toolbar` was the prefix doing a heading's job.
    let groups: Vec<(&'static str, Vec<(&'static str, AnyView)>)> = vec![
        (
            "Core",
            vec![
                ("Button", view! { <ButtonStories /> }.into_any()),
                ("Select", view! { <SelectStories /> }.into_any()),
                ("Card", view! { <CardStories /> }.into_any()),
                ("ToggleRow", view! { <ToggleRowStories /> }.into_any()),
                ("Countdown", view! { <CountdownStories /> }.into_any()),
                ("HostRow", view! { <HostRowStories /> }.into_any()),
                (
                    "SegmentedControl",
                    view! { <SegmentedControlStories /> }.into_any(),
                ),
                ("SearchInput", view! { <SearchInputStories /> }.into_any()),
                ("StateLabel", view! { <StateLabelStories /> }.into_any()),
                ("PackageRow", view! { <PackageRowStories /> }.into_any()),
                ("SkeletonBox", view! { <SkeletonStories /> }.into_any()),
                ("SplitButton", view! { <SplitButtonStories /> }.into_any()),
                ("Feedback", view! { <FeedbackStories /> }.into_any()),
                ("Forms", view! { <FormsStories /> }.into_any()),
                ("Checkbox", view! { <CheckboxStories /> }.into_any()),
                ("BackLink", view! { <BackLinkStories /> }.into_any()),
                (
                    "AnchoredOverlay",
                    view! { <AnchoredOverlayStories /> }.into_any(),
                ),
                ("ActionMenu", view! { <ActionMenuStories /> }.into_any()),
                ("PaneSection", view! { <PaneSectionStories /> }.into_any()),
                ("RevisionRow", view! { <RevisionRowStories /> }.into_any()),
                ("ChoiceGroup", view! { <ChoiceGroupStories /> }.into_any()),
                ("LoadFailure", view! { <LoadFailureStories /> }.into_any()),
                ("SelectAll", view! { <SelectAllStories /> }.into_any()),
                ("EntryRow", view! { <EntryRowStories /> }.into_any()),
                ("EntryGroup", view! { <EntryGroupStories /> }.into_any()),
            ],
        ),
        (
            "Combined",
            vec![
                ("The file list", view! { <FileListStories /> }.into_any()),
                (
                    "The list toolbar",
                    view! { <FileToolbarStories /> }.into_any(),
                ),
                ("Queue parts", view! { <QueueStories /> }.into_any()),
                (
                    "Recent files parts",
                    view! { <RecentFilesStories /> }.into_any(),
                ),
            ],
        ),
        (
            "Scenes",
            vec![
                ("The two dialogs", view! { <DialogScene /> }.into_any()),
                ("A banner in place", view! { <BannerScene /> }.into_any()),
                ("State strip", view! { <StateStripScene /> }.into_any()),
                ("Autosync paused", view! { <PausedScene /> }.into_any()),
                (
                    "A strip that could not load",
                    view! { <StripErrorScene /> }.into_any(),
                ),
                ("List toolbar", view! { <ListToolbarScene /> }.into_any()),
                ("Recent files", view! { <RecentFilesScene /> }.into_any()),
                ("Needs your attention", view! { <QueueScene /> }.into_any()),
                ("Packages", view! { <PackagesScene /> }.into_any()),
                (
                    "The package header",
                    view! { <PackageHeaderScene /> }.into_any(),
                ),
                (
                    "A check that failed",
                    view! { <UncheckedScene /> }.into_any(),
                ),
            ],
        ),
        (
            "Pages",
            vec![("Main page", view! { <PageScene /> }.into_any())],
        ),
    ];

    let index: Vec<(&'static str, Vec<&'static str>)> = groups
        .iter()
        .map(|(tier, items)| (*tier, items.iter().map(|(label, _)| *label).collect()))
        .collect();

    view! {
        <div class="g-shell">
            <nav class="g-nav" aria-label="Components">
                <Button on_click=move |_| dark.update(|d| *d = !*d)>
                    {move || if dark.get() { "Light theme" } else { "Dark theme" }}
                </Button>
                {index
                    .into_iter()
                    .map(|(tier, labels)| {
                        view! {
                            <p class="g-nav__tier">
                                <a href=format!("#{}", slug(tier))>{tier}</a>
                            </p>
                            <ul>
                                {labels
                                    .into_iter()
                                    .map(|label| {
                                        view! {
                                            <li>
                                                <a href=format!("#{}", slug(label))>{label}</a>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        }
                    })
                    .collect_view()}
            </nav>
            <main class="g-main">
                <header class="g-head">
                    <h1>"QuiltSync design system"</h1>
                    <p>
                        "Every cell is one state. Tab through them — the focus ring is
                         part of what is being reviewed."
                    </p>
                </header>
                {groups
                    .into_iter()
                    .map(|(tier, items)| {
                        view! {
                            // The tier rule, beside the tier, so a section lands in the
                            // right one without anybody opening this file.
                            <div id=slug(tier) class="g-anchor g-tier">
                                <h2 class="g-tier__name">{tier}</h2>
                                <p class="g-tier__rule">{tier_rule(tier)}</p>
                            </div>
                            {items
                                .into_iter()
                                .map(|(label, body)| {
                                    view! {
                                        <div id=slug(label) class="g-anchor">
                                            {body}
                                        </div>
                                    }
                                })
                                .collect_view()}
                        }
                    })
                    .collect_view()}
            </main>
        </div>
    }
}

/// What earns a section its tier. Rendered under the tier heading rather than
/// living only in this file's comment: the rule is for whoever is adding the
/// next section, and they are looking at the page, not at the source.
fn tier_rule(tier: &str) -> &'static str {
    match tier {
        "Core" => "One kit component, in every state it can reach. No composition.",
        "Combined" => "Several kit components wired as the kit intends. No page data.",
        "Scenes" => "One page region in one state, with fixture data.",
        _ => "A whole page at a real viewport.",
    }
}

/// Anchor id from a section label. Lossy on purpose — it only has to be stable
/// and unique across the index, not reversible.
fn slug(label: &str) -> String {
    label
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// A titled group of related states. One component may have several — `Button`
/// has plain, with-icon and large.
#[component]
#[allow(clippy::must_use_candidate, reason = "consumed by view!")]
pub fn Story(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="g-section">
            <h3>{title}</h3>
            <p class="g-note">{note}</p>
            <div class="g-grid">{children()}</div>
        </section>
    }
}

/// One labelled cell. The label is what makes the gallery reviewable: a
/// screenshot of unlabelled controls cannot be discussed.
/// A composed scene: several components arranged as the real page arranges them,
/// at the width the page gives them. Stories prove a component in isolation;
/// scenes prove they work together, which is where spacing and alignment
/// mistakes actually show up.
#[component]
#[allow(clippy::must_use_candidate, reason = "consumed by view!")]
pub fn Scene(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="g-section">
            <h3>{title}</h3>
            <p class="g-note">{note}</p>
            <div class="g-scene">{children()}</div>
        </section>
    }
}

#[component]
#[allow(clippy::must_use_candidate, reason = "consumed by view!")]
pub fn Cell(
    label: &'static str,
    /// Span two grid columns. For components that are containers — a card at one
    /// column's width reads as something it is not.
    #[prop(optional)]
    wide: bool,
    /// Span every column. For list rows, whose behaviour *is* what they do with the
    /// width they are given. Wins over `wide` if both are set.
    #[prop(optional)]
    full: bool,
    children: Children,
) -> impl IntoView {
    let class = match (full, wide) {
        (true, _) => "g-cell g-cell--full",
        (false, true) => "g-cell g-cell--wide",
        (false, false) => "g-cell",
    };
    view! {
        <div class=class>
            <span class="g-cell__label">{label}</span>
            <div class="g-cell__body">{children()}</div>
        </div>
    }
}
