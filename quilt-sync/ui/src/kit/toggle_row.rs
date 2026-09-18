//! A labelled checkbox row with a sub-label and a trailing slot.
//!
//! The two Autosync toggles are the first callers; Settings has five sections of
//! the same shape.

use leptos::prelude::*;

use super::Checkbox;

stylance::import_crate_style!(style, "src/kit/toggle_row.module.scss");

#[component]
pub fn ToggleRow(
    #[prop(into)] label: String,
    /// One line of explanation under the label. Wraps rather than truncating —
    /// it explains a setting, so losing the end of it defeats the purpose.
    #[prop(into)]
    sublabel: String,
    checked: RwSignal<bool>,
    /// A countdown, or a note like "nothing to publish". Sits *outside* the
    /// label, so clicking it does nothing — a clock is information, not a
    /// control, and flipping a setting because someone clicked a clock would be a
    /// bad surprise. That also means it may safely hold something interactive if
    /// a caller ever needs it to.
    #[prop(optional)]
    trailing: Option<AnyView>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));

    let class = move || {
        let mut out = String::from(style::root);
        if is_disabled.get() {
            out.push(' ');
            out.push_str(style::disabled);
        }
        out
    };

    view! {
        <div class=class>
            // Only this part is a label, so only this part toggles.
            <label class=style::main>
                // No `aria_label`: this `<label>` is the box's name, and a second
                // one would win over the words the reader can actually see.
                <Checkbox
                    state=Signal::derive(move || checked.get().into())
                    on_toggle=move |next| checked.set(next)
                    disabled=is_disabled
                />
                <span class=style::text>
                    <span class=style::label>{label}</span>
                    <span class=style::sublabel>{sublabel}</span>
                </span>
            </label>
            {trailing.map(|slot| view! { <span class=style::trailing>{slot}</span> })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn row(checked: RwSignal<bool>, disabled: bool) -> web_sys::Element {
        mount(move || {
            view! {
                <ToggleRow
                    label="Publish on save"
                    sublabel="Every save writes a new revision."
                    checked=checked
                    trailing=view! { <span class="clock">"in 5 min"</span> }.into_any()
                    disabled=disabled
                />
            }
        })
    }

    fn click(el: &web_sys::Element) {
        el.unchecked_ref::<web_sys::HtmlElement>().click();
    }

    fn box_of(el: &web_sys::Element) -> web_sys::HtmlInputElement {
        el.query_selector("input[type=checkbox]")
            .unwrap()
            .expect("a checkbox")
            .unchecked_into()
    }

    #[wasm_bindgen_test]
    fn clicking_the_words_toggles_the_box() {
        let checked = RwSignal::new(false);
        let el = row(checked, false);

        click(&el.query_selector("[class*=sublabel]").unwrap().unwrap());

        assert!(checked.get_untracked(), "the words are inside the label");
        assert!(box_of(&el).checked());
    }

    /// The trailing slot is information, not a control: it sits outside the
    /// `<label>` so that a click on a clock cannot flip a setting.
    #[wasm_bindgen_test]
    fn clicking_the_trailing_slot_leaves_the_box_alone() {
        let checked = RwSignal::new(false);
        let el = row(checked, false);

        click(&el.query_selector(".clock").unwrap().expect("the slot"));

        assert!(!checked.get_untracked());
        assert!(!box_of(&el).checked());
    }

    #[wasm_bindgen_test]
    fn a_disabled_row_ignores_a_click() {
        let checked = RwSignal::new(false);
        let el = row(checked, true);

        click(&el.query_selector("[class*=sublabel]").unwrap().unwrap());

        assert!(box_of(&el).disabled(), "and the input says so");
        assert!(!checked.get_untracked());
    }

    /// An `aria_label` on the box would win over the words on screen, so the row
    /// names it the only other way: by wrapping it.
    #[wasm_bindgen_test]
    fn the_box_is_named_by_the_row_rather_than_by_an_aria_label() {
        let el = row(RwSignal::new(false), false);
        let input = box_of(&el);

        assert!(input.get_attribute("aria-label").is_none());
        let labels = input.labels().expect("a label element");
        assert_eq!(labels.length(), 1, "exactly one name");
        let text = labels
            .get(0)
            .unwrap()
            .unchecked_into::<web_sys::Element>()
            .text_content()
            .unwrap_or_default();
        assert!(text.contains("Publish on save"), "got: {text}");
        assert!(
            text.contains("Every save writes a new revision."),
            "got: {text}"
        );
        assert!(
            !text.contains("in 5 min"),
            "the slot is not part of the name: {text}"
        );
    }
}
