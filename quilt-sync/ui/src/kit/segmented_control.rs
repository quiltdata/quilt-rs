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
    /// Names the group for assistive technology. Not rendered — the options are
    /// visible, so a visible group label would be redundant.
    #[prop(into)]
    label: String,
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
        <div class=style::root role="radiogroup" aria-label=label>
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
