//! Component gallery — a debug harness, and the kit's design record on GitHub
//! Pages (`deploy-gallery.yaml`). Never the app.
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
//! `just gallery preview` serves the release build, as small as the shipped
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
    pub mod button_group;
    pub mod card;
    pub mod checkbox;
    pub mod choice_group;
    pub mod commit_page;
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
    pub mod incoming_files;
    pub mod installed_package;
    pub mod json_display;
    pub mod label;
    pub mod list_toolbar;
    pub mod load_failure;
    pub mod name_tooltip;
    pub mod old_revisions_inline;
    pub mod package_autosync;
    pub mod package_header;
    pub mod package_row;
    pub mod packages;
    pub mod page;
    pub mod pane_section;
    pub mod queue;
    pub mod queue_parts;
    pub mod recent_files;
    pub mod recent_files_parts;
    pub mod revision_row;
    pub mod search_input;
    pub mod segmented_control;
    pub mod select;
    pub mod select_all;
    pub mod settings_page;
    pub mod skeleton;
    pub mod small_pages;
    pub mod split_button;
    pub mod state_label;
    pub mod state_strip;
    pub mod toggle_row;
    pub mod tooltip;
    pub mod two_columns;
    pub mod unchecked;
}

use leptos::prelude::*;

use quilt_sync_ui::story::{Catalog, Entry, Gallery, Home, HomeFrame, Tier};
// The story modules reach these as `crate::Story` and the like.
pub(crate) use quilt_sync_ui::story::{Cell, Scene, Story, after_a_beat};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <Gallery catalog=&GALLERY /> });
}

const CORE: Tier = Tier {
    name: "Core",
    slug: "core",
    rule: "One kit component, in every state it can reach. No composition.",
};
const COMBINED: Tier = Tier {
    name: "Combined",
    slug: "combined",
    rule: "Several kit components wired as the kit intends. No page data.",
};
/// Its entries, and only its entries, name the page they belong to as their area.
const SCENES: Tier = Tier {
    name: "Scenes",
    slug: "scenes",
    rule: "One page region in one state, with fixture data.",
};
const PAGES: Tier = Tier {
    name: "Pages",
    slug: "pages",
    rule: "A whole page at a real viewport.",
};

static GALLERY: Catalog = Catalog {
    tiers: &[CORE, COMBINED, SCENES, PAGES],
    entries: ENTRIES,
    home: Home {
        intro: "A short overview: the busiest frame of each of the two whole pages.",
        frames: HOME,
    },
};

/// `entry!(TIER, "Label", module::Component)`, or `entry!(SCENES / "Area", …)`.
///
/// A macro so the file name is derived from the module path rather than typed a
/// second time, and so the `use` lives inside the view it serves instead of in
/// a fifty-line block at the top of this file.
macro_rules! entry {
    ($tier:ident $(/ $area:literal)?, $label:literal, $module:ident :: $component:ident) => {
        Entry {
            tier: &$tier,
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
    entry!(CORE, "ActionMenu", action_menu::ActionMenuStories),
    entry!(
        CORE,
        "AnchoredOverlay",
        anchored_overlay::AnchoredOverlayStories
    ),
    entry!(CORE, "BackLink", back_link::BackLinkStories),
    entry!(CORE, "Button", button::ButtonStories),
    entry!(CORE, "ButtonGroup", button_group::ButtonGroupStories),
    entry!(CORE, "Card", card::CardStories),
    entry!(CORE, "Checkbox", checkbox::CheckboxStories),
    entry!(CORE, "ChoiceGroup", choice_group::ChoiceGroupStories),
    entry!(CORE, "Countdown", countdown::CountdownStories),
    entry!(CORE, "EntryGroup", entry_group::EntryGroupStories),
    entry!(CORE, "EntryRow", entry_row::EntryRowStories),
    entry!(CORE, "Feedback", feedback::FeedbackStories),
    entry!(CORE, "Forms", forms::FormsStories),
    entry!(CORE, "HostRow", host_row::HostRowStories),
    entry!(CORE, "JsonDisplay", json_display::JsonDisplayStories),
    entry!(CORE, "Label", label::LabelStories),
    entry!(CORE, "LoadFailure", load_failure::LoadFailureStories),
    entry!(CORE, "NameTooltip", name_tooltip::NameTooltipStories),
    entry!(CORE, "PackageRow", package_row::PackageRowStories),
    entry!(CORE, "PaneSection", pane_section::PaneSectionStories),
    entry!(CORE, "RevisionRow", revision_row::RevisionRowStories),
    entry!(CORE, "SearchInput", search_input::SearchInputStories),
    entry!(
        CORE,
        "SegmentedControl",
        segmented_control::SegmentedControlStories
    ),
    entry!(CORE, "Select", select::SelectStories),
    entry!(CORE, "SelectAll", select_all::SelectAllStories),
    entry!(CORE, "SkeletonBox", skeleton::SkeletonStories),
    entry!(CORE, "SplitButton", split_button::SplitButtonStories),
    entry!(CORE, "StateLabel", state_label::StateLabelStories),
    entry!(CORE, "ToggleRow", toggle_row::ToggleRowStories),
    entry!(CORE, "Tooltip", tooltip::TooltipStories),
    entry!(CORE, "TwoColumns", two_columns::TwoColumnsStories),
    entry!(COMBINED, "File list", file_list::FileListStories),
    entry!(
        COMBINED,
        "File pane toolbar",
        file_toolbar::FileToolbarStories
    ),
    entry!(COMBINED, "Queue parts", queue_parts::QueueStories),
    entry!(
        COMBINED,
        "Recent files parts",
        recent_files_parts::RecentFilesStories
    ),
    entry!(
        SCENES / "Main page",
        "State strip",
        state_strip::StateStripScene
    ),
    entry!(
        SCENES / "Main page",
        "Autosync paused",
        state_strip::PausedScene
    ),
    entry!(
        SCENES / "Main page",
        "A strip that could not load",
        state_strip::StripErrorScene
    ),
    entry!(
        SCENES / "Main page",
        "Autosync per package",
        package_autosync::PerPackageScene
    ),
    entry!(
        SCENES / "Main page",
        "A banner in place",
        feedback::BannerScene
    ),
    entry!(
        SCENES / "Main page",
        "Needs your attention",
        queue::QueueScene
    ),
    entry!(
        SCENES / "Main page",
        "A check that failed",
        unchecked::UncheckedScene
    ),
    entry!(
        SCENES / "Main page",
        "List toolbar",
        list_toolbar::ListToolbarScene
    ),
    entry!(
        SCENES / "Main page",
        "Recent files",
        recent_files::RecentFilesScene
    ),
    entry!(SCENES / "Main page", "Packages", packages::PackagesScene),
    entry!(
        SCENES / "Installed package",
        "Package header",
        package_header::PackageHeaderScene
    ),
    entry!(
        SCENES / "Installed package",
        "Context pane",
        context_pane::ContextPaneScene
    ),
    entry!(
        SCENES / "Installed package",
        "File pane",
        file_pane::FilePaneScene
    ),
    entry!(
        SCENES / "Old revisions",
        "Remove old revisions",
        old_revisions_inline::OldRevisionsInlineScene
    ),
    entry!(SCENES / "Dialogs", "The three dialogs", forms::DialogScene),
    entry!(
        SCENES / "Dialogs",
        "A dialog that was refused",
        forms::RefusedScene
    ),
    entry!(
        SCENES / "Dialogs",
        "A confirmation",
        confirm_dialog::ConfirmScene
    ),
    entry!(PAGES, "Main page", page::PageScene),
    entry!(
        PAGES,
        "Installed package",
        installed_package::InstalledPackageScene
    ),
    entry!(PAGES, "Commit page", commit_page::CommitPageScene),
    entry!(PAGES, "Settings page", settings_page::SettingsPageScene),
    entry!(PAGES, "Small pages", small_pages::SmallPagesScene),
    entry!(
        PAGES,
        "Newer revision available",
        incoming_files::IncomingFilesScene
    ),
    entry!(PAGES, "While loading", page::LoadingScene),
];

/// The busiest frame of each of the two whole pages. `#/` used to be every
/// section, so the first load mounted all of them; even the two `Pages`
/// sections are ten frames. These two are what is looked at most, and the rest
/// is one link away at `#/all`.
const HOME: &[HomeFrame] = &[
    HomeFrame {
        tier: &PAGES,
        from: "Main page",
        view: || {
            use crate::gallery::page::BusyDayScene;
            view! { <BusyDayScene /> }.into_any()
        },
    },
    HomeFrame {
        tier: &PAGES,
        from: "Installed package",
        view: || {
            use crate::gallery::installed_package::SelectingScene;
            view! { <SelectingScene /> }.into_any()
        },
    },
];

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

#[cfg(test)]
mod tests {
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use quilt_sync_ui::story::{Gallery, Route};

    use super::{ENTRIES, GALLERY, HOME, SCENES};
    use crate::gallery::entry_group::EntryGroupStories;
    use crate::gallery::entry_row::EntryRowStories;
    use crate::gallery::file_pane::FilePaneScene;
    use crate::gallery::installed_package::InstalledPackageScene;

    #[test]
    fn every_section_is_reachable_at_its_own_address() {
        for (i, entry) in ENTRIES.iter().enumerate() {
            assert_eq!(
                Route::parse(&GALLERY, &entry.href()),
                Route::Entry(i),
                "{}",
                entry.label
            );
        }
    }

    #[test]
    fn a_tier_address_shows_the_tier() {
        for tier in GALLERY.tiers {
            assert_eq!(
                Route::parse(&GALLERY, &format!("#/{}", tier.slug)),
                Route::Tier(tier)
            );
        }
    }

    /// One frame from each whole page, and each names a real `Pages` entry, so a
    /// renamed label fails here rather than at the first load.
    #[test]
    fn each_overview_frame_comes_from_a_page() {
        let sources: Vec<_> = HOME.iter().map(|f| f.source(&GALLERY).href()).collect();
        assert_eq!(sources, ["#/pages/main-page", "#/pages/installed-package"]);
        for frame in HOME {
            let found = ENTRIES
                .iter()
                .filter(|e| e.tier == frame.tier && e.label == frame.from)
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
        let handle =
            leptos::mount::mount_to(container.clone(), || quilt_sync_ui::story::home(&GALLERY));
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
        let handle = leptos::mount::mount_to(container.clone(), || {
            view! { <Gallery catalog=&GALLERY /> }
        });
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
        hashes.extend(GALLERY.tiers.iter().map(|t| format!("/{}", t.slug)));
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
    fn no_tier_is_called_all() {
        for tier in GALLERY.tiers {
            assert_ne!(tier.slug, "all");
        }
    }

    #[test]
    fn scenes_and_only_scenes_have_a_page() {
        for entry in ENTRIES {
            assert_eq!(
                entry.area.is_some(),
                *entry.tier == SCENES,
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

    /// The failed-check cell starts collapsed, as every cell does, and opens
    /// itself: the field's own rule, not the fixture's. Its primary is the
    /// one disabled; the resting cell's is not.
    #[wasm_bindgen_test]
    async fn the_failed_check_opens_its_own_section() {
        use crate::gallery::commit_page::CommitPageScene;

        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), CommitPageScene);
        leptos::task::tick().await;
        let has = |selector: &str| container.query_selector(selector).unwrap().is_some();
        let failed_open = has("#commit-failed-check textarea");
        let rest_open = has("#commit-rest textarea");
        let failed_blocked = has("#commit-failed-check [data-primary-action] button[disabled]");
        let rest_blocked = has("#commit-rest [data-primary-action] button[disabled]");
        drop(handle);
        container.remove();
        assert!(failed_open, "a failed check opens the metadata editor");
        assert!(!rest_open, "a passing one leaves it folded");
        assert!(failed_blocked && !rest_blocked);
    }

    /// The cells that are about an error draw it: a minute field below 1, one
    /// not whole, and metadata that is not JSON, whose Save is disabled.
    #[wasm_bindgen_test]
    async fn the_settings_scene_draws_its_errors() {
        use crate::gallery::settings_page::SettingsPageScene;

        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), SettingsPageScene);
        leptos::task::tick().await;
        let text = container.text_content().unwrap_or_default();
        let invalid = container
            .query_selector_all("input[type=number][aria-invalid=true]")
            .unwrap()
            .length();
        drop(handle);
        container.remove();
        assert!(text.contains("At least 1 minute"), "the minute error");
        assert!(
            text.contains("Whole minutes only"),
            "the narrow cell's error"
        );
        assert_eq!(invalid, 2, "only the two error cells' fields are invalid");
        assert!(text.contains("Not valid JSON"), "the metadata error");
    }

    #[wasm_bindgen_test]
    fn every_description_the_installed_package_scene_names_is_drawn() {
        assert_eq!(
            dangling_descriptions(InstalledPackageScene),
            Vec::<String>::new()
        );
    }
}
