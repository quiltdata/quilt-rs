//! A single-line text field.
//!
//! Separate from [`SearchInput`](super::SearchInput) rather than a variant of it. A search
//! field has a magnifier, a clear button, `type="search"`, and its value is a filter the
//! user expects to discard; this is a value being *entered*, which is why it has none of
//! those and can be invalid. Same reasoning that kept the two row components apart: the
//! job differs, not the weight.
//!
//! It carries no label of its own. [`FormControl`](super::FormControl) does, and hands this the ids
//! that connect the two — which is also why a `TextInput` outside a `FormControl` does not
//! compile.

use leptos::prelude::*;

use super::ControlId;

stylance::import_crate_style!(style, "src/kit/text_input.module.scss");

#[component]
pub fn TextInput(
    /// From the [`FormControl`](super::FormControl) that labels this input. Required, and
    /// its only source is `FormControl`'s `control` closure — see [`ControlId`] for why
    /// that is deliberate.
    id: ControlId,
    value: RwSignal<String>,
    /// An example of the shape wanted — `owner/package-name`, `my-s3-bucket`. Never the
    /// label: a placeholder disappears the moment the user types, so a field labelled only
    /// by its placeholder becomes anonymous exactly when it holds data.
    #[prop(optional, into)]
    placeholder: String,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Draws the error border and sets `aria-invalid`. The *message* is `FormControl`'s, because
    /// only the caller knows what is wrong; this only knows that something is.
    #[prop(optional, into)]
    invalid: MaybeProp<bool>,
    /// Focused on mount — for the first field of a dialog, where the user opened the thing
    /// in order to type.
    #[prop(optional)]
    autofocus: bool,
) -> impl IntoView {
    let (control_id, described_by) = id.into_attrs();
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
            id=control_id
            aria-describedby=described_by
            placeholder=placeholder
            autofocus=autofocus
            prop:value=move || value.get()
            prop:disabled=move || is_disabled.get()
            aria-invalid=move || is_invalid.get().then_some("true")
            on:input=move |ev| value.set(event_target_value(&ev))
        />
    }
}

#[cfg(test)]
mod tests {
    use super::super::FormControl;
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn field(el: &web_sys::Element, selector: &str) -> web_sys::HtmlInputElement {
        el.query_selector(selector)
            .unwrap()
            .expect("the field")
            .dyn_into()
            .unwrap()
    }

    /// Typing reaches the caller's signal. Without it the field is decoration.
    #[wasm_bindgen_test]
    async fn typing_writes_the_callers_signal() {
        let value = RwSignal::new(String::new());
        let el = mount(move || {
            view! {
                <FormControl label="Package name" control=move |id| {
                    view! { <TextInput id=id value=value /> }.into_any()
                } />
            }
        });
        let input = field(&el, "input");
        input.set_value("acme/demo");
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
        assert_eq!(value.get_untracked(), "acme/demo");
    }

    /// `invalid` draws the border AND says so; the message is `FormControl`'s, because
    /// only the caller knows what is wrong.
    #[wasm_bindgen_test]
    fn an_invalid_field_says_so_as_well_as_drawing_it() {
        let value = RwSignal::new(String::new());
        let el = mount(move || {
            view! {
                <FormControl label="Package name" control=move |id| {
                    view! { <TextInput id=id value=value invalid=true /> }.into_any()
                } />
            }
        });
        let input = field(&el, "input");
        assert_eq!(input.get_attribute("aria-invalid").as_deref(), Some("true"));
        assert!(input.class_name().contains("invalid"), "and it is drawn");
    }

    /// A placeholder disappears the moment the user types, so it is never the label.
    #[wasm_bindgen_test]
    fn a_field_is_named_by_its_label_and_not_its_placeholder() {
        let value = RwSignal::new(String::new());
        let el = mount(move || {
            view! {
                <FormControl label="Package name" control=move |id| {
                    view! { <TextInput id=id value=value placeholder="owner/name" /> }.into_any()
                } />
            }
        });
        let input = field(&el, "input");
        assert_eq!(
            input.get_attribute("placeholder").as_deref(),
            Some("owner/name")
        );
        let label = el.query_selector("label").unwrap().unwrap();
        assert_eq!(
            label.get_attribute("for").as_deref(),
            input.get_attribute("id").as_deref(),
            "the name comes from the label"
        );
    }
}
