//! Segmented control — one choice, all options visible.
//!
//! The same data as [`Select`](super::Select): a list of strings and the chosen
//! one. The difference is only whether the options are on screen or behind a
//! dropdown, so the rule of thumb is a count — two or three short options here,
//! more than that in a `Select`.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/segmented_control.module.scss");

#[component]
pub fn SegmentedControl(
    /// Names the group for assistive technology. Never drawn — the options are visible, so
    /// a visible group label would be redundant.
    ///
    /// `aria_label` and not a [`FormControl`](super::FormControl): this only ever appears in a toolbar,
    /// where every option is already on screen. Same reason `SearchInput` takes one.
    #[prop(into)]
    aria_label: String,
    /// Groups the radios. Must be unique on the page — two controls sharing a name
    /// become one group, and selecting in either clears the other.
    ///
    /// `&'static str` deliberately: a name that varied at runtime would silently
    /// regroup the inputs, so it is not something a caller should be able to
    /// compute.
    name: &'static str,
    options: Vec<String>,
    selected: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div class=style::root role="radiogroup" aria-label=aria_label>
            {options
                .into_iter()
                .map(|option| {
                    let value = option.clone();
                    let is_selected = {
                        let value = value.clone();
                        move || selected.get() == value
                    };
                    let on_change = {
                        let value = value.clone();
                        move |_| selected.set(value.clone())
                    };
                    view! {
                        <label class=style::option>
                            <input
                                type="radio"
                                class=style::input
                                name=name
                                value=value
                                prop:checked=is_selected
                                on:change=on_change
                            />
                            <span class=style::text>{option}</span>
                        </label>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::element_saying;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn control(selected: RwSignal<String>) -> web_sys::Element {
        mount(move || {
            view! {
                <SegmentedControl
                    aria_label="Group files by"
                    name="group-by"
                    options=vec!["Flat".to_string(), "Package".to_string()]
                    selected=selected
                />
            }
        })
    }

    fn radios(el: &web_sys::Element) -> Vec<web_sys::HtmlInputElement> {
        let found = el.query_selector_all("input").unwrap();
        (0..found.length())
            .map(|i| found.get(i).unwrap().unchecked_into())
            .collect()
    }

    /// The options are on screen, so the group carries no visible heading — which
    /// leaves `aria-label` as the only thing naming what the choice is about.
    #[wasm_bindgen_test]
    fn the_group_is_named_for_a_reader_who_cannot_see_it() {
        let el = control(RwSignal::new("Flat".to_string()));
        let group = el
            .query_selector("[role=radiogroup]")
            .unwrap()
            .expect("a group");
        assert_eq!(
            group.get_attribute("aria-label").as_deref(),
            Some("Group files by")
        );
    }

    /// One `name` across the options is what makes them one choice rather than
    /// several independent ones.
    #[wasm_bindgen_test]
    fn the_options_are_radios_in_one_group() {
        let el = control(RwSignal::new("Flat".to_string()));
        let radios = radios(&el);

        assert_eq!(radios.len(), 2, "one per option");
        for radio in &radios {
            assert_eq!(radio.type_(), "radio");
            assert_eq!(radio.name(), "group-by");
        }
        assert!(radios[0].checked(), "the selected one");
        assert!(!radios[1].checked());
    }

    #[wasm_bindgen_test]
    fn clicking_an_option_s_word_selects_it() {
        let selected = RwSignal::new("Flat".to_string());
        let el = control(selected);

        element_saying(&el, "Package").click();

        assert_eq!(
            selected.get_untracked(),
            "Package",
            "the word is inside its label"
        );
        assert!(radios(&el)[1].checked());
    }

    /// The toolbar writes this signal too — a control that only followed its own
    /// clicks would show the wrong option after one.
    #[wasm_bindgen_test]
    async fn a_selection_made_elsewhere_moves_the_checked_option() {
        let selected = RwSignal::new("Flat".to_string());
        let el = control(selected);

        selected.set("Package".to_string());
        leptos::task::tick().await;

        let radios = radios(&el);
        assert!(!radios[0].checked());
        assert!(radios[1].checked());
    }
}
