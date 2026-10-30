//! A few lines of free text, such as what went wrong in a problem report.
//!
//! [`TextInput`](super::TextInput)'s box and its rules: no label of its own,
//! so it takes the [`ControlId`] a [`FormControl`](super::FormControl) hands
//! it, and a `TextArea` outside one does not compile.

use leptos::prelude::*;

use super::ControlId;

// `TextInput`'s `invalid` has no use here: nothing validates free text.
stylance::import_crate_style!(
    #[allow(dead_code)]
    field,
    "src/kit/text_input.module.scss"
);
stylance::import_crate_style!(style, "src/kit/text_area.module.scss");

#[component]
pub fn TextArea(
    id: ControlId,
    value: RwSignal<String>,
    /// Visible lines before it scrolls; the reader can drag it taller.
    #[prop(default = 4)]
    rows: u32,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let (control_id, described_by) = id.into_attrs();
    view! {
        <textarea
            class=format!("{} {}", field::root, style::root)
            id=control_id
            aria-describedby=described_by
            rows=rows
            placeholder=placeholder
            prop:value=move || value.get()
            prop:disabled=move || disabled.get().unwrap_or(false)
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

    #[wasm_bindgen_test]
    async fn typing_writes_the_callers_signal() {
        let value = RwSignal::new(String::new());
        let el = mount(move || {
            view! {
                <FormControl label="What went wrong?" control=move |id| {
                    view! { <TextArea id=id value=value /> }.into_any()
                } />
            }
        });
        let area: web_sys::HtmlTextAreaElement = el
            .query_selector("textarea")
            .unwrap()
            .expect("the field")
            .dyn_into()
            .unwrap();
        area.set_value("It stopped syncing");
        area.dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
        assert_eq!(value.get_untracked(), "It stopped syncing");
    }
}
