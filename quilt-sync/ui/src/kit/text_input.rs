//! A single-line text field.
//!
//! Separate from [`SearchInput`](super::SearchInput) rather than a variant of it. A search
//! field has a magnifier, a clear button, `type="search"`, and its value is a filter the
//! user expects to discard; this is a value being *entered*, which is why it has none of
//! those and can be invalid. Same reasoning that kept the two row components apart: the
//! job differs, not the weight.
//!
//! It carries no label. [`Field`](super::Field) does — see `qhq-kt31`.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/text_input.module.scss");

#[component]
pub fn TextInput(
    value: RwSignal<String>,
    /// An example of the shape wanted — `owner/package-name`, `my-s3-bucket`. Never the
    /// label: a placeholder disappears the moment the user types, so a field labelled only
    /// by its placeholder becomes anonymous exactly when it holds data.
    #[prop(optional, into)]
    placeholder: String,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Draws the error border and sets `aria-invalid`. The *message* is `Field`'s, because
    /// only the caller knows what is wrong; this only knows that something is.
    #[prop(optional, into)]
    invalid: MaybeProp<bool>,
    /// Focused on mount — for the first field of a dialog, where the user opened the thing
    /// in order to type.
    #[prop(optional)]
    autofocus: bool,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));
    let is_invalid = Signal::derive(move || invalid.get().unwrap_or(false));

    let class = move || {
        if is_invalid.get() {
            format!("{} {}", style::root, style::invalid)
        } else {
            style::root.to_string()
        }
    };

    view! {
        <input
            type="text"
            class=class
            placeholder=placeholder
            autofocus=autofocus
            prop:value=move || value.get()
            prop:disabled=move || is_disabled.get()
            aria-invalid=move || is_invalid.get().then_some("true")
            on:input=move |ev| value.set(event_target_value(&ev))
        />
    }
}
