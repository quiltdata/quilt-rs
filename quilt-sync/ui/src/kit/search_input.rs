//! Search field.
//!
//! Filters what is already on screen; it does not fetch. So it holds a signal the
//! caller reads, and has no notion of results, loading or emptiness — an empty
//! result is the list's message to write, not this control's.

use leptos::prelude::*;

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
                                <svg
                                    viewBox="0 0 11 11"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="1.6"
                                    stroke-linecap="round"
                                >
                                    <path d="M2 2 9 9M9 2 2 9" />
                                </svg>
                            </button>
                        }
                    })
            }}
        </div>
    }
}
