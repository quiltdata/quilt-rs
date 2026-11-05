//! The kit's storybook — every component in every state, and the design record
//! on GitHub Pages (`deploy-gallery.yaml`). Never the app.
//!
//! Run it with `just storybook` and iterate in Chrome or Firefox for speed —
//! then check **GNOME Web (Epiphany)**, which is `WebKitGTK`, before committing a
//! layout. Chrome is not the webview that ships on Linux. The app's gallery
//! (`quilt-sync/ui/src/gallery.rs`) shows the app's regions and pages.
//!
//! # Render states, not components
//!
//! One entry per *state*. A storybook listing `Button` once is useless; listing
//! its twelve is a visual-regression surface. Add a cell for every state a
//! component can reach, including the ones that only appear when something has
//! gone wrong.

// This bin is its own compilation unit, so the library's `configure!` does not
// reach tests compiled here.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use leptos::prelude::*;

use quilt_ui_kit::story::{Catalog, Entry, Gallery, Home, Tier};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <Gallery catalog=&STORYBOOK /> });
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

static STORYBOOK: Catalog = Catalog {
    title: "QuiltSync UI kit",
    elsewhere: ("the app's gallery", "../"),
    tiers: &[CORE, COMBINED],
    entries: ENTRIES,
    home: Home {
        intro: "The kit's components, one state per cell, and the parts they combine into.",
        frames: &[],
    },
};

/// `entry!(TIER, "Label", module::Component)`.
///
/// A macro so the file name is derived from the module path rather than typed a
/// second time.
macro_rules! entry {
    ($tier:ident, $label:literal, $module:ident :: $component:ident) => {
        Entry {
            tier: &$tier,
            area: None,
            label: $label,
            file: concat!(stringify!($module), ".rs"),
            view: || {
                use quilt_ui_kit::stories::$module::$component;
                view! { <$component /> }.into_any()
            },
        }
    };
}

/// Every section, alphabetical within its tier, since they are looked up by name.
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
    entry!(CORE, "Inbox", inbox::InboxStories),
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
    entry!(CORE, "UpdateBand", update_band::UpdateBandStories),
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
];

#[cfg(test)]
mod tests {
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use quilt_ui_kit::stories::entry_group::EntryGroupStories;
    use quilt_ui_kit::stories::entry_row::EntryRowStories;
    use quilt_ui_kit::story::{Gallery, Route};

    use super::{ENTRIES, STORYBOOK};

    #[test]
    fn every_section_is_reachable_at_its_own_address() {
        for (i, entry) in ENTRIES.iter().enumerate() {
            assert_eq!(
                Route::parse(&STORYBOOK, &entry.href()),
                Route::Entry(i),
                "{}",
                entry.label
            );
        }
    }

    #[test]
    fn no_tier_is_called_all() {
        for tier in STORYBOOK.tiers {
            assert_ne!(tier.slug, "all");
        }
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

    /// Every id an `aria-describedby` in `container` names that is not drawn
    /// exactly once there, except a form control's validation message while it
    /// has none to show: `form_control.rs` names that id from the start on
    /// purpose, so a message that appears later is announced.
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

    /// Every address the storybook has draws each id once, and every description
    /// a row names is drawn once beside it.
    #[wasm_bindgen_test]
    async fn every_route_draws_each_id_once_and_every_description_once() {
        let mut hashes = vec!["/".to_string(), "/all".to_string()];
        hashes.extend(STORYBOOK.tiers.iter().map(|t| format!("/{}", t.slug)));
        hashes.extend(
            ENTRIES
                .iter()
                .map(|e| e.href().trim_start_matches('#').to_string()),
        );
        let window = web_sys::window().unwrap();
        let doc = window.document().unwrap();
        window.location().set_hash("/").unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), || {
            view! { <Gallery catalog=&STORYBOOK /> }
        });
        let go = |hash: &str| {
            window.location().set_hash(hash).unwrap();
            window
                .dispatch_event(&web_sys::Event::new("hashchange").unwrap())
                .unwrap();
        };
        let mut wrong = Vec::new();
        for hash in &hashes {
            go(hash);
            leptos::task::tick().await;
            let mut twice: Vec<String> = ids_in(&container)
                .windows(2)
                .filter(|w| w[0] == w[1])
                .map(|w| w[0].clone())
                .collect();
            twice.dedup();
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

    /// Every id a story's `aria-describedby` names that the story itself does
    /// not draw.
    fn dangling_descriptions<N: IntoView + 'static>(
        story: impl FnOnce() -> N + 'static,
    ) -> Vec<String> {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), story);
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
}
