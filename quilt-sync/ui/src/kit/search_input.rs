//! Search field.
//!
//! Filters what is already on screen; it does not fetch. So it holds a signal the
//! caller reads, and has no notion of results, loading or emptiness — an empty
//! result is the list's message to write, not this control's.

use leptos::prelude::*;

use super::icons;

stylance::import_crate_style!(style, "src/kit/search_input.module.scss");

#[component]
pub fn SearchInput(
    value: RwSignal<String>,
    /// Accessible name, never drawn. A search field with only a placeholder is unlabelled
    /// once the user starts typing, because the placeholder disappears.
    ///
    /// `aria_label` and not a [`FormControl`](super::FormControl), unlike `TextInput`: this only ever
    /// appears in a toolbar, where a stacked label would cost a line to say what the
    /// magnifier and the placeholder already say. Same reason `SegmentedControl` takes one.
    #[prop(into)]
    aria_label: String,
    #[prop(into, optional)] placeholder: String,
) -> impl IntoView {
    view! {
        <div class=style::root>
            <span class=style::icon aria-hidden="true">
                <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5"
                    stroke-linecap="round">
                    <circle cx="6" cy="6" r="4.25" />
                    <path d="M9.25 9.25 12.5 12.5" />
                </svg>
            </span>
            <input
                type="search"
                class=style::field
                aria-label=aria_label
                placeholder=placeholder
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
            {move || {
                (!value.get().is_empty())
                    .then(|| {
                        view! {
                            <button
                                type="button"
                                class=style::clear
                                aria-label="Clear search"
                                title="Clear search"
                                on:click=move |_| value.set(String::new())
                            >
                                {icons::x()}
                            </button>
                        }
                    })
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// Nothing to clear until there is something, so the control is not there either.
    #[wasm_bindgen_test]
    async fn the_clear_control_appears_only_with_something_to_clear() {
        let value = RwSignal::new(String::new());
        let el = mount(move || view! { <SearchInput value=value aria_label="Search packages" /> });
        assert!(
            el.query_selector("[class*=clear]").unwrap().is_none(),
            "an empty field offers nothing"
        );

        value.set("rna".to_string());
        leptos::task::tick().await;
        assert!(el.query_selector("[class*=clear]").unwrap().is_some());
    }

    #[wasm_bindgen_test]
    async fn clearing_empties_the_callers_signal() {
        let value = RwSignal::new("rna".to_string());
        let el = mount(move || view! { <SearchInput value=value aria_label="Search packages" /> });
        let clear: web_sys::HtmlElement = el
            .query_selector("[class*=clear]")
            .unwrap()
            .expect("the clear control")
            .dyn_into()
            .unwrap();
        clear.click();
        leptos::task::tick().await;
        assert_eq!(value.get_untracked(), "");
    }

    /// The placeholder disappears the moment the user types, so the name cannot be it.
    #[wasm_bindgen_test]
    fn the_field_carries_a_name_that_is_never_drawn() {
        let value = RwSignal::new(String::new());
        let el = mount(move || {
            view! { <SearchInput value=value aria_label="Search packages" placeholder="Search…" /> }
        });
        let input = el.query_selector("input").unwrap().expect("the field");
        assert_eq!(
            input.get_attribute("aria-label").as_deref(),
            Some("Search packages")
        );
    }
}
