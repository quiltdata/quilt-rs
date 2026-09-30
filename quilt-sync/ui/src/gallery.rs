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
    pub mod old_revisions_dialog;
    pub mod old_revisions_inline;
    pub mod old_revisions_keep_n;
    pub mod old_revisions_settings;
    pub mod package_header;
    pub mod package_size;
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
        Scenes / "Old revisions (options)",
        "Package size in the context pane",
        package_size::PackageSizeScene
    ),
    entry!(
        Scenes / "Old revisions (options)",
        "Option A · remove from the popover",
        old_revisions_inline::OldRevisionsInlineScene
    ),
    entry!(
        Scenes / "Old revisions (options)",
        "Option B · a Manage revisions dialog",
        old_revisions_dialog::OldRevisionsDialogScene
    ),
    entry!(
        Scenes / "Old revisions (options)",
        "Option C · Settings Storage section",
        old_revisions_settings::OldRevisionsSettingsScene
    ),
    entry!(
        Scenes / "Old revisions (options)",
        "Option D · keep the last N automatically",
        old_revisions_keep_n::OldRevisionsKeepNScene
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

/// What the URL fragment asks for.
///
/// A hash route rather than `leptos_router`: the gallery is one static page
/// under `trunk serve`, and a fragment needs neither a server fallback nor a
/// router context the stories would then be rendered inside.
///
/// Routes rather than tabs. Tabs would show one component at a time and cost
/// the thing a design-system gallery is *for*: noticing that a Select is a pixel
/// taller than a Button, or that two components disagree about a baseline. So
/// `#/` is still the one long scroll, and a tier (`#/core`) is still a page to
/// compare across and Ctrl+F through — the narrower routes are for looking
/// something up, and for not mounting two whole-page scenes to look at a Button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
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
            Route::Unknown => false,
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
                        <a href="#/" aria-current=current(Route::All)>"All"</a>
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
                                <a href="#/">"Show every section."</a>
                            </p>
                        }
                            .into_any();
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
        // The tier rule, beside the tier, so a section lands in the right one
        // without anybody opening this file.
        <div class="g-tier">
            <h2 class="g-tier__name">{tier.name()}</h2>
            <p class="g-tier__rule">{tier.rule()}</p>
        </div>
        {sections}
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
/// pane. A marked row's `aria-describedby` names `DIFFERS_ID`, which in the app
/// only the pane carries; a cell without it would point at nothing.
/// `count` is how many rows the cell marks, which the sentence counts.
#[must_use]
pub fn differs_caption(count: usize) -> AnyView {
    view! { <p class="g-note" id=kit::DIFFERS_ID>{pages::differs_sentence(count)}</p> }.into_any()
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

    use super::{ENTRIES, Route, Tier};
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
    fn no_fragment_shows_everything() {
        for hash in ["", "#", "#/"] {
            assert_eq!(Route::parse(hash), Route::All, "{hash:?}");
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
