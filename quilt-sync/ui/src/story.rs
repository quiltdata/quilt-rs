//! The gallery's harness: the shell, the routes and the story wrappers. A
//! gallery binary declares its tiers, its entries and its overview as a
//! [`Catalog`] and mounts [`Gallery`] with it; the stories themselves stay in
//! the binary.
//!
//! It draws with `kit` and the shared `theme` switch only, so it renders in a
//! plain browser with no Tauri runtime.

use leptos::ev::SubmitEvent;
use leptos::prelude::*;

use crate::kit::{Button, SearchInput};
use crate::theme;

/// What a section IS, not where it sits in a menu. The tier is the heading, so
/// a section's own label does not repeat it — `Scene · list toolbar` was the
/// prefix doing a heading's job.
#[derive(Debug, PartialEq, Eq)]
pub struct Tier {
    pub name: &'static str,
    /// The route segment. Never `all`, which is the long scroll's address.
    pub slug: &'static str,
    /// What earns a section its tier. Rendered under the tier heading rather
    /// than living only in the source: the rule is for whoever is adding the
    /// next section, and they are looking at the page, not at the source.
    pub rule: &'static str,
}

/// One section of the gallery.
pub struct Entry {
    pub tier: &'static Tier,
    /// The page a scene's region belongs to, so the scenes read as the pages
    /// they make up rather than as one flat list.
    pub area: Option<&'static str>,
    pub label: &'static str,
    /// The story module, shown beside the section and matched by the filter: a
    /// label that reads as prose ("Needs your attention") still leads to a file
    /// (`queue.rs`).
    pub file: &'static str,
    pub view: fn() -> AnyView,
}

impl Entry {
    pub fn slug(&self) -> String {
        slug(self.label)
    }

    pub fn href(&self) -> String {
        format!("#/{}/{}", self.tier.slug, self.slug())
    }

    /// Case-insensitive, over the label and the file. `query` is lower-cased by
    /// the caller, once per keystroke rather than once per entry.
    fn matches(&self, query: &str) -> bool {
        self.label.to_lowercase().contains(query) || self.file.contains(query)
    }
}

/// One frame of `#/`: a single scene lifted out of a section.
pub struct HomeFrame {
    pub tier: &'static Tier,
    /// The label of the entry in `tier` the frame comes from. Its source link
    /// goes there, so the rest of that section is one click away.
    pub from: &'static str,
    pub view: fn() -> AnyView,
}

impl HomeFrame {
    /// # Panics
    ///
    /// When no entry in `tier` has the label `from`.
    pub fn source(&self, catalog: &'static Catalog) -> &'static Entry {
        catalog
            .entries
            .iter()
            .find(|e| e.tier == self.tier && e.label == self.from)
            .expect("a home frame names an entry")
    }
}

/// What `#/` shows: a few frames and none of the gallery's sections, so the
/// first load does not mount all of them.
pub struct Home {
    /// The overview's opening words, before the links to every section and
    /// to each tier.
    pub intro: &'static str,
    pub frames: &'static [HomeFrame],
}

/// Everything one gallery shows.
pub struct Catalog {
    /// In page order.
    pub tiers: &'static [Tier],
    /// Every section, in page order within its tier.
    pub entries: &'static [Entry],
    pub home: Home,
}

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
/// `#/` is not the long scroll: mounting every section made the first load slow.
/// The bare address shows the catalog's [`Home`], with a line pointing at
/// everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// `#/`: the home frames, and no section.
    Home,
    /// `#/all`: every section.
    All,
    Tier(&'static Tier),
    /// An index into the catalog's entries.
    Entry(usize),
    /// A fragment that names nothing, such as a link from before the routes.
    Unknown,
}

impl Route {
    pub fn parse(catalog: &'static Catalog, hash: &str) -> Route {
        let path = hash.trim_start_matches('#').trim_matches('/');
        if path.is_empty() {
            return Route::Home;
        }
        if path == "all" {
            return Route::All;
        }
        let (tier, section) = match path.split_once('/') {
            Some((tier, section)) => (tier, Some(section)),
            None => (path, None),
        };
        let Some(tier) = catalog.tiers.iter().find(|t| t.slug == tier) else {
            return Route::Unknown;
        };
        let Some(section) = section else {
            return Route::Tier(tier);
        };
        catalog
            .entries
            .iter()
            .position(|e| e.tier == tier && e.slug() == section)
            .map_or(Route::Unknown, Route::Entry)
    }

    fn current(catalog: &'static Catalog) -> Route {
        Route::parse(catalog, &window().location().hash().unwrap_or_default())
    }

    pub fn shows(self, catalog: &'static Catalog, index: usize) -> bool {
        match self {
            Route::All => true,
            Route::Tier(tier) => catalog.entries[index].tier == tier,
            Route::Entry(i) => i == index,
            // Home draws its frames instead, which share ids with the sections
            // they come from: mounting both would duplicate them.
            Route::Home | Route::Unknown => false,
        }
    }
}

fn go(href: &str) {
    // `set_hash` fires `hashchange`, and the listener in `Gallery` does the rest.
    let _ = window().location().set_hash(href);
}

#[component]
pub fn Gallery(catalog: &'static Catalog) -> impl IntoView {
    let entries = catalog.entries;
    let dark = RwSignal::new(false);

    // The app's own switch, shared: the gallery drives it from a button and
    // the app from the OS, and one of them getting the root element wrong is
    // exactly the bug the shared version's doc explains.
    Effect::new(move |_| theme::set(dark.get()));
    // A v2 reader, so v2-only rules apply — the loading frame's ground is one.
    theme::set_v2(true);

    let route = RwSignal::new(Route::current(catalog));
    let handle = window_event_listener(leptos::ev::hashchange, move |_| {
        route.set(Route::current(catalog));
        // A new route is a new page. Keeping the old scroll offset would land
        // halfway down one section because the last one was long.
        window().scroll_to_with_x_and_y(0.0, 0.0);
    });
    on_cleanup(move || handle.remove());

    let filter = RwSignal::new(String::new());
    let matching = move || {
        let query = filter.get().trim().to_lowercase();
        (0..entries.len())
            .filter(|&i| entries[i].matches(&query))
            .collect::<Vec<_>>()
    };
    // Enter opens the first match, so the filter is also a jump box.
    let on_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if let Some(&first) = matching().first() {
            go(&entries[first].href());
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
                        catalog
                            .tiers
                            .iter()
                            .filter_map(|tier| {
                                let items: Vec<usize> = matching
                                    .iter()
                                    .copied()
                                    .filter(|&i| entries[i].tier == tier)
                                    .collect();
                                (!items.is_empty())
                                    .then(|| nav_tier(entries, tier, &items, current))
                            })
                            .collect_view()
                            .into_any()
                    }}
                </div>
            </nav>
            <main class="g-main">
                {masthead()}
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
                        return home(catalog);
                    }
                    catalog
                        .tiers
                        .iter()
                        .filter_map(|tier| {
                            let items: Vec<usize> = (0..entries.len())
                                .filter(|&i| entries[i].tier == tier && route.shows(catalog, i))
                                .collect();
                            (!items.is_empty()).then(|| main_tier(entries, tier, &items))
                        })
                        .collect_view()
                        .into_any()
                }}
            </main>
        </div>
    }
}

fn masthead() -> impl IntoView {
    view! {
        <header class="g-head">
            // Public on Pages: a reader landing from a search must not take it
            // for the app.
            <p class="g-head__record">
                "Component gallery — the design record of QuiltSync, not the app."
            </p>
            <h1>"QuiltSync design system"</h1>
            <p>
                "Every cell is one state. Tab through them — the focus ring is
                 part of what is being reviewed."
            </p>
        </header>
    }
}

/// `#/`: the overview line, then the home frames under the tier headings they
/// come from. Each frame is built here, so no other route mounts it.
pub fn home(catalog: &'static Catalog) -> AnyView {
    // Said on the page, so a short first load does not read as a gallery that
    // lost most of its sections.
    let overview = view! {
        <p class="g-note">
            {catalog.home.intro}
            " "
            <a href="#/all">"Every section"</a>
            " is one long scroll; a tier — "
            {catalog
                .tiers
                .iter()
                .enumerate()
                .map(|(n, tier)| {
                    view! {
                        {(n > 0).then_some(", ")}
                        <a href=format!("#/{}", tier.slug)>{tier.name}</a>
                    }
                })
                .collect_view()}
            " — is a shorter one."
        </p>
    };
    let mut tier = None;
    let mut frames = Vec::new();
    for frame in catalog.home.frames {
        if tier != Some(frame.tier) {
            tier = Some(frame.tier);
            frames.push(tier_heading(frame.tier));
        }
        let source = frame.source(catalog);
        frames.push(
            view! {
                <div class="g-entry">
                    <a class="g-entry__source" href=source.href()>
                        "src/gallery/"
                        {source.file}
                    </a>
                    {(frame.view)()}
                </div>
            }
            .into_any(),
        );
    }
    view! {
        {overview}
        {frames}
    }
    .into_any()
}

/// One tier of the index: its heading, then its sections, with a scene's page
/// as a sub-heading wherever it changes.
fn nav_tier<F>(
    entries: &'static [Entry],
    tier: &'static Tier,
    items: &[usize],
    current: impl Fn(Route) -> F,
) -> AnyView
where
    F: Fn() -> Option<&'static str> + Send + Sync + 'static,
{
    let mut area = None;
    let mut list = Vec::new();
    for &i in items {
        let entry = &entries[i];
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
            <a href=format!("#/{}", tier.slug) aria-current=current(Route::Tier(tier))>
                {tier.name}
            </a>
        </p>
        <ul>{list}</ul>
    }
    .into_any()
}

/// One tier of the page: the heading and its rule, then the sections the route
/// shows. Each section is built here, so a route that does not show it never
/// mounts it.
fn main_tier(entries: &'static [Entry], tier: &'static Tier, items: &[usize]) -> AnyView {
    let mut area = None;
    let mut sections = Vec::new();
    for &i in items {
        let entry = &entries[i];
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

fn tier_heading(tier: &'static Tier) -> AnyView {
    view! {
        // The tier rule, beside the tier, so a section lands in the right one
        // without anybody opening the source.
        <div class="g-tier">
            <h2 class="g-tier__name">{tier.name}</h2>
            <p class="g-tier__rule">{tier.rule}</p>
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
pub fn Story(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="g-section">
            <h3>{title}</h3>
            <p class="g-note">{note}</p>
            <div class="g-grid">{children()}</div>
        </section>
    }
}

/// A composed scene: several components arranged as the real page arranges them,
/// at the width the page gives them. Stories prove a component in isolation;
/// scenes prove they work together, which is where spacing and alignment
/// mistakes actually show up.
#[component]
pub fn Scene(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="g-section">
            <h3>{title}</h3>
            <p class="g-note">{note}</p>
            <div class="g-scene">{children()}</div>
        </section>
    }
}

/// One labelled cell. The label is what makes the gallery reviewable: a
/// screenshot of unlabelled controls cannot be discussed.
#[component]
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

    use super::{Catalog, Entry, Home, Route, Tier};

    const CORE: Tier = Tier {
        name: "Core",
        slug: "core",
        rule: "",
    };
    const SCENES: Tier = Tier {
        name: "Scenes",
        slug: "scenes",
        rule: "",
    };

    fn nothing() -> AnyView {
        ().into_any()
    }

    static CATALOG: Catalog = Catalog {
        tiers: &[CORE, SCENES],
        entries: &[
            Entry {
                tier: &CORE,
                area: None,
                label: "Button",
                file: "button.rs",
                view: nothing,
            },
            Entry {
                tier: &SCENES,
                area: Some("Main page"),
                label: "List toolbar",
                file: "list_toolbar.rs",
                view: nothing,
            },
        ],
        home: Home {
            intro: "",
            frames: &[],
        },
    };

    #[test]
    fn every_section_is_reachable_at_its_own_address() {
        for (i, entry) in CATALOG.entries.iter().enumerate() {
            assert_eq!(Route::parse(&CATALOG, &entry.href()), Route::Entry(i));
        }
    }

    #[test]
    fn a_tier_address_shows_the_tier() {
        for tier in CATALOG.tiers {
            for hash in [format!("#/{}", tier.slug), format!("#/{}/", tier.slug)] {
                assert_eq!(Route::parse(&CATALOG, &hash), Route::Tier(tier));
            }
        }
    }

    #[test]
    fn no_fragment_is_the_overview() {
        for hash in ["", "#", "#/"] {
            assert_eq!(Route::parse(&CATALOG, hash), Route::Home, "{hash:?}");
        }
    }

    /// Not one of the sections: the home frames are drawn on their own, and
    /// share ids with the sections they come from.
    #[test]
    fn the_overview_mounts_no_section() {
        assert!((0..CATALOG.entries.len()).all(|i| !Route::Home.shows(&CATALOG, i)));
    }

    #[test]
    fn the_all_address_shows_everything() {
        for hash in ["#/all", "#/all/"] {
            assert_eq!(Route::parse(&CATALOG, hash), Route::All, "{hash:?}");
        }
        assert!((0..CATALOG.entries.len()).all(|i| Route::All.shows(&CATALOG, i)));
    }

    #[test]
    fn a_tier_address_shows_only_its_tier() {
        let core = Route::parse(&CATALOG, "#/core");
        assert!(core.shows(&CATALOG, 0));
        assert!(!core.shows(&CATALOG, 1));
    }

    /// The anchors the gallery had before it had routes, and addresses that
    /// name a real tier but no section in it.
    #[test]
    fn an_address_that_names_nothing_is_unknown() {
        for hash in ["#button", "#/buttons", "#/core/nope", "#/scenes/button"] {
            assert_eq!(Route::parse(&CATALOG, hash), Route::Unknown, "{hash:?}");
        }
    }
}
