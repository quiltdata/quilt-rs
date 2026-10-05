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
//! `just gallery-release` serves a release build, as small as the shipped
//! app's, for a final look; its builds are slower.
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
pub(crate) use quilt_sync_ui::{commands, pages};

// One module per component. Adding a story means adding a file here and one
// `entry!` line in `ENTRIES` below — the nav, the routes and the page all read
// that one list.
mod gallery {
    pub mod action_menu;
    pub mod anchored_overlay;
    pub mod back_link;
    pub mod button;
    pub mod card;
    pub mod checkbox;
    pub mod choice_group;
    pub mod confirm_dialog;
    pub mod context_pane;
    pub mod countdown;
    pub mod entry_group;
    pub mod entry_row;
    pub mod feedback;
    pub mod file_list;
    pub mod file_pane;
    pub mod file_toolbar;
    pub mod forms;
    pub mod host_row;
    pub mod installed_package;
    pub mod list_toolbar;
    pub mod load_failure;
    pub mod old_revisions_inline;
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

use leptos::ev::SubmitEvent;
use leptos::prelude::*;

use kit::{Button, SearchInput};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(Gallery);
}

/// What a section IS, not where it sits in a menu. The tier is the heading, so
/// a section's own label does not repeat it — `Scene · list toolbar` was the
/// prefix doing a heading's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    Core,
    Combined,
    Scenes,
    Pages,
}

impl Tier {
    const ALL: [Tier; 4] = [Tier::Core, Tier::Combined, Tier::Scenes, Tier::Pages];

    fn name(self) -> &'static str {
        match self {
            Tier::Core => "Core",
            Tier::Combined => "Combined",
            Tier::Scenes => "Scenes",
            Tier::Pages => "Pages",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Tier::Core => "core",
            Tier::Combined => "combined",
            Tier::Scenes => "scenes",
            Tier::Pages => "pages",
        }
    }

    /// What earns a section its tier. Rendered under the tier heading rather
    /// than living only in this file: the rule is for whoever is adding the next
    /// section, and they are looking at the page, not at the source.
    fn rule(self) -> &'static str {
        match self {
            Tier::Core => "One kit component, in every state it can reach. No composition.",
            Tier::Combined => "Several kit components wired as the kit intends. No page data.",
            Tier::Scenes => "One page region in one state, with fixture data.",
            Tier::Pages => "A whole page at a real viewport.",
        }
    }
}

/// One section of the gallery.
struct Entry {
    tier: Tier,
    /// The page a scene's region belongs to, so the scenes read as the pages
    /// they make up rather than as one flat list. `None` outside `Scenes`.
    area: Option<&'static str>,
    label: &'static str,
    /// The story module, shown beside the section and matched by the filter: a
    /// label that reads as prose ("Needs your attention") still leads to a file
    /// (`queue.rs`).
    file: &'static str,
    view: fn() -> AnyView,
}

impl Entry {
    fn slug(&self) -> String {
        slug(self.label)
    }

    fn href(&self) -> String {
        format!("#/{}/{}", self.tier.slug(), self.slug())
    }

    /// Case-insensitive, over the label and the file. `query` is lower-cased by
    /// the caller, once per keystroke rather than once per entry.
    fn matches(&self, query: &str) -> bool {
        self.label.to_lowercase().contains(query) || self.file.contains(query)
    }
}

/// `entry!(Tier, "Label", module::Component)`, or `entry!(Scenes / "Area", …)`.
///
/// A macro so the file name is derived from the module path rather than typed a
/// second time, and so the `use` lives inside the view it serves instead of in
/// a fifty-line block at the top of this file.
macro_rules! entry {
    ($tier:ident $(/ $area:literal)?, $label:literal, $module:ident :: $component:ident) => {
        Entry {
            tier: Tier::$tier,
            area: entry!(@area $($area)?),
            label: $label,
            file: concat!(stringify!($module), ".rs"),
            view: || {
                use crate::gallery::$module::$component;
                view! { <$component /> }.into_any()
            },
        }
    };
    (@area) => { None };
    (@area $area:literal) => { Some($area) };
}

/// Every section, in page order. Core and Combined are alphabetical, since they
/// are looked up by name; Scenes are grouped by the page they belong to, then
/// in the order that page draws them top to bottom.
const ENTRIES: &[Entry] = &[
    entry!(Core, "ActionMenu", action_menu::ActionMenuStories),
    entry!(
        Core,
        "AnchoredOverlay",
        anchored_overlay::AnchoredOverlayStories
    ),
    entry!(Core, "BackLink", back_link::BackLinkStories),
    entry!(Core, "Button", button::ButtonStories),
    entry!(Core, "Card", card::CardStories),
    entry!(Core, "Checkbox", checkbox::CheckboxStories),
    entry!(Core, "ChoiceGroup", choice_group::ChoiceGroupStories),
    entry!(Core, "Countdown", countdown::CountdownStories),
    entry!(Core, "EntryGroup", entry_group::EntryGroupStories),
    entry!(Core, "EntryRow", entry_row::EntryRowStories),
    entry!(Core, "Feedback", feedback::FeedbackStories),
    entry!(Core, "Forms", forms::FormsStories),
    entry!(Core, "HostRow", host_row::HostRowStories),
    entry!(Core, "LoadFailure", load_failure::LoadFailureStories),
    entry!(Core, "PackageRow", packages::PackageRowStories),
    entry!(Core, "PaneSection", pane_section::PaneSectionStories),
    entry!(Core, "RevisionRow", revision_row::RevisionRowStories),
    entry!(Core, "SearchInput", search_input::SearchInputStories),
    entry!(
        Core,
        "SegmentedControl",
        segmented_control::SegmentedControlStories
    ),
    entry!(Core, "Select", select::SelectStories),
    entry!(Core, "SelectAll", select_all::SelectAllStories),
    entry!(Core, "SkeletonBox", skeleton::SkeletonStories),
    entry!(Core, "SplitButton", split_button::SplitButtonStories),
    entry!(Core, "StateLabel", state_label::StateLabelStories),
    entry!(Core, "ToggleRow", toggle_row::ToggleRowStories),
    entry!(Combined, "File list", file_list::FileListStories),
    entry!(
        Combined,
        "File pane toolbar",
        file_toolbar::FileToolbarStories
    ),
    entry!(Combined, "Queue parts", queue::QueueStories),
    entry!(
        Combined,
        "Recent files parts",
        recent_files::RecentFilesStories
    ),
    entry!(
        Scenes / "Main page",
        "State strip",
        state_strip::StateStripScene
    ),
    entry!(
        Scenes / "Main page",
        "Autosync paused",
        state_strip::PausedScene
    ),
    entry!(
        Scenes / "Main page",
        "A strip that could not load",
        state_strip::StripErrorScene
    ),
    entry!(
        Scenes / "Main page",
        "A banner in place",
        feedback::BannerScene
    ),
    entry!(
        Scenes / "Main page",
        "Needs your attention",
        queue::QueueScene
    ),
    entry!(
        Scenes / "Main page",
        "A check that failed",
        unchecked::UncheckedScene
    ),
    entry!(
        Scenes / "Main page",
        "List toolbar",
        list_toolbar::ListToolbarScene
    ),
    entry!(
        Scenes / "Main page",
        "Recent files",
        recent_files::RecentFilesScene
    ),
    entry!(Scenes / "Main page", "Packages", packages::PackagesScene),
    entry!(
        Scenes / "Installed package",
        "Package header",
        package_header::PackageHeaderScene
    ),
    entry!(
        Scenes / "Installed package",
        "Context pane",
        context_pane::ContextPaneScene
    ),
    entry!(
        Scenes / "Installed package",
        "File pane",
        file_pane::FilePaneScene
    ),
    entry!(
        Scenes / "Old revisions",
        "Remove old revisions",
        old_revisions_inline::OldRevisionsInlineScene
    ),
    entry!(Scenes / "Dialogs", "The three dialogs", forms::DialogScene),
    entry!(
        Scenes / "Dialogs",
        "A dialog that was refused",
        forms::RefusedScene
    ),
    entry!(
        Scenes / "Dialogs",
        "A confirmation",
        confirm_dialog::ConfirmScene
    ),
    entry!(Pages, "Main page", page::PageScene),
    entry!(
        Pages,
        "Installed package",
        installed_package::InstalledPackageScene
    ),
    entry!(Pages, "While loading", page::LoadingScene),
];

/// One frame of `#/`: a single scene lifted out of a `Pages` section.
struct HomeFrame {
    /// The label of the `Pages` entry the frame comes from. Its source link goes
    /// there, so the rest of that page's frames are one click away.
    from: &'static str,
    view: fn() -> AnyView,
}

impl HomeFrame {
    fn source(&self) -> &'static Entry {
        ENTRIES
            .iter()
            .find(|e| e.tier == Tier::Pages && e.label == self.from)
            .expect("a home frame names a Pages entry")
    }
}

/// What `#/` shows: the busiest frame of each of the two whole pages, and none
/// of the gallery's sections. `#/` used to be every section, so the first load
/// mounted all of them; even the two `Pages` sections are ten frames. These two
/// are what is looked at most, and the rest is one link away at `#/all`.
const HOME: &[HomeFrame] = &[
    HomeFrame {
        from: "Main page",
        view: || {
            use crate::gallery::page::BusyDayScene;
            view! { <BusyDayScene /> }.into_any()
        },
    },
    HomeFrame {
        from: "Installed package",
        view: || {
            use crate::gallery::installed_package::SelectingScene;
            view! { <SelectingScene /> }.into_any()
        },
    },
];

/// What the URL fragment asks for.
///
/// A hash route rather than `leptos_router`: the gallery is one static page
/// under `trunk serve`, and a fragment needs neither a server fallback nor a
/// router context the stories would then be rendered inside.
///
/// Routes rather than tabs. Tabs would show one component at a time and cost
/// the thing a design-system gallery is *for*: noticing that a Select is a pixel
/// taller than a Button, or that two components disagree about a baseline. So
/// `#/all` is still the one long scroll, and a tier (`#/core`) is still a page to
/// compare across and Ctrl+F through — the narrower routes are for looking
/// something up, and for not mounting two whole-page scenes to look at a Button.
///
/// `#/` is not the long scroll: mounting every section made the first load slow,
/// and what is opened first is nearly always one of the two whole pages. So the
/// bare address shows `HOME`, the busiest frame of each, with a line pointing at
/// everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// `#/`: the frames in `HOME`, and no section of `ENTRIES`.
    Home,
    /// `#/all`: every section.
    All,
    Tier(Tier),
    /// An index into `ENTRIES`.
    Entry(usize),
    /// A fragment that names nothing, such as a link from before the routes.
    Unknown,
}

impl Route {
    fn parse(hash: &str) -> Route {
        let path = hash.trim_start_matches('#').trim_matches('/');
        if path.is_empty() {
            return Route::Home;
        }
        // `all` cannot shadow a tier: a test holds the tier slugs to that.
        if path == "all" {
            return Route::All;
        }
        let (tier, section) = match path.split_once('/') {
            Some((tier, section)) => (tier, Some(section)),
            None => (path, None),
        };
        let Some(tier) = Tier::ALL.into_iter().find(|t| t.slug() == tier) else {
            return Route::Unknown;
        };
        let Some(section) = section else {
            return Route::Tier(tier);
        };
        ENTRIES
            .iter()
            .position(|e| e.tier == tier && e.slug() == section)
            .map_or(Route::Unknown, Route::Entry)
    }

    fn current() -> Route {
        Route::parse(&window().location().hash().unwrap_or_default())
    }

    fn shows(self, index: usize) -> bool {
        match self {
            Route::All => true,
            Route::Tier(tier) => ENTRIES[index].tier == tier,
            Route::Entry(i) => i == index,
            // Home draws `HOME`'s frames instead, which share ids with the
            // sections they come from (`page-selecting`): mounting both would
            // duplicate them.
            Route::Home | Route::Unknown => false,
        }
    }
}

fn go(href: &str) {
    // `set_hash` fires `hashchange`, and the listener in `Gallery` does the rest.
    let _ = window().location().set_hash(href);
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

    let route = RwSignal::new(Route::current());
    let handle = window_event_listener(leptos::ev::hashchange, move |_| {
        route.set(Route::current());
        // A new route is a new page. Keeping the old scroll offset would land
        // halfway down one section because the last one was long.
        window().scroll_to_with_x_and_y(0.0, 0.0);
    });
    on_cleanup(move || handle.remove());

    let filter = RwSignal::new(String::new());
    let matching = move || {
        let query = filter.get().trim().to_lowercase();
        (0..ENTRIES.len())
            .filter(|&i| ENTRIES[i].matches(&query))
            .collect::<Vec<_>>()
    };
    // Enter opens the first match, so the filter is also a jump box.
    let on_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if let Some(&first) = matching().first() {
            go(&ENTRIES[first].href());
        }
    };

    let current = move |r: Route| move || (route.get() == r).then_some("page");

    view! {
        <div class="g-shell">
            <nav class="g-nav" aria-label="Components">
                <Button on_click=move |_| dark.update(|d| *d = !*d)>
                    {move || if dark.get() { "Light theme" } else { "Dark theme" }}
                </Button>
                <form class="g-nav__filter" role="search" on:submit=on_submit>
                    <SearchInput
                        value=filter
                        aria_label="Filter sections"
                        placeholder="Filter"
                    />
                </form>
                // The index scrolls, the theme toggle and the filter do not: the nav
                // is taller than a short window long before the gallery is finished,
                // and a control that scrolls out of a pinned sidebar is a control
                // nobody finds again.
                <div class="g-nav__index">
                    <p class="g-nav__tier">
                        <a href="#/" aria-current=current(Route::Home)>"Overview"</a>
                    </p>
                    <p class="g-nav__tier">
                        <a href="#/all" aria-current=current(Route::All)>"All"</a>
                    </p>
                    {move || {
                        let matching = matching();
                        let filtering = !filter.get().trim().is_empty();
                        if filtering && matching.is_empty() {
                            return view! { <p class="g-nav__empty">"No section matches."</p> }
                                .into_any();
                        }
                        Tier::ALL
                            .into_iter()
                            .filter_map(|tier| {
                                let items: Vec<usize> = matching
                                    .iter()
                                    .copied()
                                    .filter(|&i| ENTRIES[i].tier == tier)
                                    .collect();
                                (!items.is_empty()).then(|| nav_tier(tier, &items, current))
                            })
                            .collect_view()
                            .into_any()
                    }}
                </div>
            </nav>
            <main class="g-main">
                <header class="g-head">
                    <h1>"QuiltSync design system"</h1>
                    <p>
                        "Every cell is one state. Tab through them — the focus ring is
                         part of what is being reviewed."
                    </p>
                </header>
                {move || {
                    let route = route.get();
                    if route == Route::Unknown {
                        return view! {
                            <p class="g-note">
                                "Nothing in the gallery is at this address. "
                                <a href="#/all">"Show every section."</a>
                            </p>
                        }
                            .into_any();
                    }
                    if route == Route::Home {
                        return home().into_any();
                    }
                    Tier::ALL
                        .into_iter()
                        .filter_map(|tier| {
                            let items: Vec<usize> = (0..ENTRIES.len())
                                .filter(|&i| ENTRIES[i].tier == tier && route.shows(i))
                                .collect();
                            (!items.is_empty()).then(|| main_tier(tier, &items))
                        })
                        .collect_view()
                        .into_any()
                }}
            </main>
        </div>
    }
}

/// `#/`: the overview line, then `HOME`'s frames under the `Pages` heading they
/// come from. Each frame is built here, so no other route mounts it.
fn home() -> AnyView {
    // Said on the page, so a short first load does not read as a gallery that
    // lost most of its sections.
    let overview = view! {
        <p class="g-note">
            "A short overview: the busiest frame of each of the two whole pages. "
            <a href="#/all">"Every section"</a>
            " is one long scroll; a tier — "
            {Tier::ALL
                .into_iter()
                .enumerate()
                .map(|(n, tier)| {
                    view! {
                        {(n > 0).then_some(", ")}
                        <a href=format!("#/{}", tier.slug())>{tier.name()}</a>
                    }
                })
                .collect_view()}
            " — is a shorter one."
        </p>
    };
    let frames = HOME
        .iter()
        .map(|frame| {
            let source = frame.source();
            view! {
                <div class="g-entry">
                    <a class="g-entry__source" href=source.href()>
                        "src/gallery/"
                        {source.file}
                    </a>
                    {(frame.view)()}
                </div>
            }
        })
        .collect_view();
    view! {
        {overview}
        {tier_heading(Tier::Pages)}
        {frames}
    }
    .into_any()
}

/// One tier of the index: its heading, then its sections, with a scene's page
/// as a sub-heading wherever it changes.
fn nav_tier<F>(tier: Tier, items: &[usize], current: impl Fn(Route) -> F) -> AnyView
where
    F: Fn() -> Option<&'static str> + Send + Sync + 'static,
{
    let mut area = None;
    let mut list = Vec::new();
    for &i in items {
        let entry = &ENTRIES[i];
        if entry.area.is_some() && entry.area != area {
            area = entry.area;
            list.push(view! { <li class="g-nav__area">{area}</li> }.into_any());
        }
        list.push(
            view! {
                <li>
                    <a href=entry.href() title=entry.file aria-current=current(Route::Entry(i))>
                        {entry.label}
                    </a>
                </li>
            }
            .into_any(),
        );
    }
    view! {
        <p class="g-nav__tier">
            <a href=format!("#/{}", tier.slug()) aria-current=current(Route::Tier(tier))>
                {tier.name()}
            </a>
        </p>
        <ul>{list}</ul>
    }
    .into_any()
}

/// One tier of the page: the heading and its rule, then the sections the route
/// shows. Each section is built here, so a route that does not show it never
/// mounts it.
fn main_tier(tier: Tier, items: &[usize]) -> AnyView {
    let mut area = None;
    let mut sections = Vec::new();
    for &i in items {
        let entry = &ENTRIES[i];
        if entry.area.is_some() && entry.area != area {
            area = entry.area;
            sections.push(view! { <p class="g-area">{area}</p> }.into_any());
        }
        sections.push(
            view! {
                <div class="g-entry">
                    <a class="g-entry__source" href=entry.href()>
                        "src/gallery/"
                        {entry.file}
                    </a>
                    {(entry.view)()}
                </div>
            }
            .into_any(),
        );
    }
    view! {
        {tier_heading(tier)}
        {sections}
    }
    .into_any()
}

fn tier_heading(tier: Tier) -> AnyView {
    view! {
        // The tier rule, beside the tier, so a section lands in the right one
        // without anybody opening this file.
        <div class="g-tier">
            <h2 class="g-tier__name">{tier.name()}</h2>
            <p class="g-tier__rule">{tier.rule()}</p>
        </div>
    }
    .into_any()
}

/// Route segment from a section label. Lossy on purpose — it only has to be
/// stable and unique within a tier, not reversible.
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

/// The resolve pane's sentence, for a cell that draws marked rows without the
/// pane. A marked row's `aria-describedby` names the sentence, which in the app
/// only the pane carries; a cell without it would point at nothing.
/// `count` is how many rows the cell marks, which the sentence counts.
///
/// Its id is the cell's own `kit::DiffersId`, read the same way the rows read
/// it, so a cell that forgets to provide one draws `DIFFERS_ID` and the
/// gallery's unique-id test names it.
#[must_use]
pub fn differs_caption(count: usize) -> AnyView {
    view! { <p class="g-note" id=kit::differs_id()>{pages::differs_sentence(count)}</p> }.into_any()
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

#[cfg(test)]
mod tests {
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{ENTRIES, HOME, Route, Tier};
    use crate::gallery::entry_group::EntryGroupStories;
    use crate::gallery::entry_row::EntryRowStories;
    use crate::gallery::file_pane::FilePaneScene;
    use crate::gallery::installed_package::InstalledPackageScene;

    #[test]
    fn every_section_is_reachable_at_its_own_address() {
        for (i, entry) in ENTRIES.iter().enumerate() {
            assert_eq!(
                Route::parse(&entry.href()),
                Route::Entry(i),
                "{}",
                entry.label
            );
        }
    }

    #[test]
    fn a_tier_address_shows_the_tier() {
        for tier in Tier::ALL {
            assert_eq!(
                Route::parse(&format!("#/{}", tier.slug())),
                Route::Tier(tier)
            );
            assert_eq!(
                Route::parse(&format!("#/{}/", tier.slug())),
                Route::Tier(tier)
            );
        }
    }

    #[test]
    fn no_fragment_is_the_overview() {
        for hash in ["", "#", "#/"] {
            assert_eq!(Route::parse(hash), Route::Home, "{hash:?}");
        }
    }

    /// Not one of the sections: `HOME`'s frames are drawn on their own, and
    /// share ids with the sections they come from.
    #[test]
    fn the_overview_mounts_no_section() {
        assert!((0..ENTRIES.len()).all(|i| !Route::Home.shows(i)));
    }

    /// One frame from each whole page, and each names a real `Pages` entry, so a
    /// renamed label fails here rather than at the first load.
    #[test]
    fn each_overview_frame_comes_from_a_page() {
        let sources: Vec<_> = HOME.iter().map(|f| f.source().href()).collect();
        assert_eq!(sources, ["#/pages/main-page", "#/pages/installed-package"]);
        for frame in HOME {
            let found = ENTRIES
                .iter()
                .filter(|e| e.tier == Tier::Pages && e.label == frame.from)
                .count();
            assert_eq!(found, 1, "{}", frame.from);
        }
    }

    /// The overview as it mounts: its two frames together, with no id drawn twice.
    #[wasm_bindgen_test]
    fn the_overview_draws_no_id_twice() {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), super::home);
        let with_id = container.query_selector_all("[id]").unwrap();
        let mut ids = Vec::new();
        for i in 0..with_id.length() {
            let el: web_sys::Element = with_id.item(i).unwrap().dyn_into().unwrap();
            ids.push(el.id());
        }
        let frames = container.query_selector_all(".g-entry").unwrap().length();
        drop(handle);
        container.remove();
        assert_eq!(frames, 2);
        assert!(ids.iter().any(|id| id == "page-selecting"), "{ids:?}");
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
    }

    /// The ids of everything in `container` that has one, sorted.
    fn ids_in(container: &web_sys::Element) -> Vec<String> {
        let with_id = container.query_selector_all("[id]").unwrap();
        let mut ids: Vec<String> = (0..with_id.length())
            .map(|i| {
                let el: web_sys::Element = with_id.item(i).unwrap().dyn_into().unwrap();
                el.id()
            })
            .collect();
        ids.sort();
        ids
    }

    /// The gallery, mounted on `#/`, and a way to move it to another route
    /// through its own `hashchange` listener. Dispatched by hand so the swap
    /// does not wait on the browser's own event, which arrives a task later and
    /// only sets the same route again.
    fn mount_gallery() -> (web_sys::HtmlElement, impl Drop, impl Fn(&str)) {
        let window = web_sys::window().unwrap();
        let doc = window.document().unwrap();
        window.location().set_hash("/").unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), super::Gallery);
        let go = move |hash: &str| {
            window.location().set_hash(hash).unwrap();
            window
                .dispatch_event(&web_sys::Event::new("hashchange").unwrap())
                .unwrap();
        };
        (container, handle, go)
    }

    /// Every id `container` draws more than once.
    fn ids_twice(container: &web_sys::Element) -> Vec<String> {
        let mut twice: Vec<String> = ids_in(container)
            .windows(2)
            .filter(|w| w[0] == w[1])
            .map(|w| w[0].clone())
            .collect();
        twice.dedup();
        twice
    }

    /// Every id an `aria-describedby` in `container` names that is not drawn
    /// exactly once there, so the description is either missing or ambiguous.
    ///
    /// Except a form control's validation message while it has none to show:
    /// `kit/form_control.rs` names that id from the start on purpose, so a
    /// message that appears later is announced, and a missing id is ignored.
    fn descriptions_not_drawn_once(container: &web_sys::Element) -> Vec<String> {
        let unshown_validation = |id: &str| id.starts_with("q-control-") && id.ends_with("-error");
        let ids = ids_in(container);
        let described = container.query_selector_all("[aria-describedby]").unwrap();
        let mut wrong = Vec::new();
        for i in 0..described.length() {
            let el: web_sys::Element = described.item(i).unwrap().dyn_into().unwrap();
            for id in el
                .get_attribute("aria-describedby")
                .unwrap()
                .split_whitespace()
            {
                let drawn = ids.iter().filter(|drawn| *drawn == id).count();
                if drawn > 1 || (drawn == 0 && !unshown_validation(id)) {
                    wrong.push(id.to_string());
                }
            }
        }
        wrong.sort();
        wrong.dedup();
        wrong
    }

    /// The overview's frames share ids with the sections they come from, so
    /// the swap between `#/` and `#/all` must take one set down before it puts
    /// the other up.
    #[wasm_bindgen_test]
    async fn switching_between_the_overview_and_everything_draws_no_id_twice() {
        let (container, handle, go) = mount_gallery();
        let mut seen = Vec::new();
        for hash in ["", "/all", "/"] {
            if !hash.is_empty() {
                go(hash);
            }
            leptos::task::tick().await;
            let ids = ids_in(&container);
            let selecting = ids.iter().filter(|id| *id == "page-selecting").count();
            let frames = container.query_selector_all(".g-entry").unwrap().length() as usize;
            seen.push((hash, frames, selecting, ids_twice(&container)));
        }
        drop(handle);
        container.remove();
        go("");
        // (route, frames drawn, how many `page-selecting`, the ids drawn twice)
        let none = Vec::<String>::new;
        assert_eq!(
            seen,
            [
                ("", 2, 1, none()),
                ("/all", ENTRIES.len(), 1, none()),
                ("/", 2, 1, none()),
            ]
        );
    }

    /// Every address the gallery has draws each id once, and every description
    /// a row names is drawn once beside it. The long scroll is the hard case:
    /// several cells mark rows and draw the resolve sentence, and each pairs
    /// its rows with its own sentence through `kit::DiffersId`.
    #[wasm_bindgen_test]
    async fn every_route_draws_each_id_once_and_every_description_once() {
        let mut hashes = vec!["/".to_string(), "/all".to_string()];
        hashes.extend(Tier::ALL.iter().map(|t| format!("/{}", t.slug())));
        hashes.extend(
            ENTRIES
                .iter()
                .map(|e| e.href().trim_start_matches('#').to_string()),
        );
        let (container, handle, go) = mount_gallery();
        let mut wrong = Vec::new();
        for hash in &hashes {
            go(hash);
            leptos::task::tick().await;
            let twice = ids_twice(&container);
            let described = descriptions_not_drawn_once(&container);
            if !twice.is_empty() || !described.is_empty() {
                wrong.push((hash.clone(), twice, described));
            }
        }
        drop(handle);
        container.remove();
        go("");
        assert_eq!(wrong, Vec::new());
    }

    #[test]
    fn the_all_address_shows_everything() {
        for hash in ["#/all", "#/all/"] {
            assert_eq!(Route::parse(hash), Route::All, "{hash:?}");
        }
        assert!((0..ENTRIES.len()).all(|i| Route::All.shows(i)));
    }

    #[test]
    fn no_tier_is_called_all() {
        for tier in Tier::ALL {
            assert_ne!(tier.slug(), "all");
        }
    }

    /// The anchors the gallery had before it had routes, and addresses that
    /// name a real tier but no section in it.
    #[test]
    fn an_address_that_names_nothing_is_unknown() {
        for hash in ["#button", "#/buttons", "#/core/nope", "#/scenes/button"] {
            assert_eq!(Route::parse(hash), Route::Unknown, "{hash:?}");
        }
    }

    #[test]
    fn scenes_and_only_scenes_have_a_page() {
        for entry in ENTRIES {
            assert_eq!(
                entry.area.is_some(),
                entry.tier == Tier::Scenes,
                "{}",
                entry.label
            );
        }
    }

    /// Every id a scene's `aria-describedby` names, and whether the scene itself
    /// draws it. Scoped to the scene, because the gallery is one page and another
    /// scene's sentence would otherwise answer for this one's rows.
    fn dangling_descriptions<N: IntoView + 'static>(
        scene: impl FnOnce() -> N + 'static,
    ) -> Vec<String> {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), scene);
        let described = container.query_selector_all("[aria-describedby]").unwrap();
        let mut missing = Vec::new();
        for i in 0..described.length() {
            let el: web_sys::Element = described.item(i).unwrap().dyn_into().unwrap();
            let ids = el.get_attribute("aria-describedby").unwrap();
            for id in ids.split_whitespace() {
                if container
                    .query_selector(&format!("[id='{id}']"))
                    .unwrap()
                    .is_none()
                {
                    missing.push(id.to_string());
                }
            }
        }
        drop(handle);
        container.remove();
        missing
    }

    #[wasm_bindgen_test]
    fn every_description_the_entry_row_stories_name_is_drawn() {
        assert_eq!(dangling_descriptions(EntryRowStories), Vec::<String>::new());
    }

    #[wasm_bindgen_test]
    fn every_description_the_entry_group_stories_name_is_drawn() {
        assert_eq!(
            dangling_descriptions(EntryGroupStories),
            Vec::<String>::new()
        );
    }

    #[wasm_bindgen_test]
    fn every_description_the_file_pane_scene_names_is_drawn() {
        assert_eq!(dangling_descriptions(FilePaneScene), Vec::<String>::new());
    }

    /// The skeletons are the app's, and the app's appbar navigates: a scene that
    /// reached for a router would panic on mount here, where there is none. Both
    /// pages drawn, each a v2 surface with a region that says it is busy.
    #[wasm_bindgen_test]
    fn the_loading_scene_draws_both_pages_without_a_router() {
        use crate::gallery::page::LoadingScene;

        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), LoadingScene);
        let pages = container
            .query_selector_all("[data-v2-page]")
            .unwrap()
            .length();
        let busy = container
            .query_selector_all("[data-v2-page] [aria-busy=true]")
            .unwrap()
            .length();
        drop(handle);
        container.remove();
        assert_eq!(pages, 2, "the main page and the package page");
        assert!(busy >= 2, "each says it is busy, found {busy}");
    }

    #[wasm_bindgen_test]
    fn every_description_the_installed_package_scene_names_is_drawn() {
        assert_eq!(
            dangling_descriptions(InstalledPackageScene),
            Vec::<String>::new()
        );
    }
}
