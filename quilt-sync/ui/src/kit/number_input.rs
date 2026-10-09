//! A whole-number field, a few digits wide, for a value that sits inside a
//! sentence: Settings' `Every [1] minutes`.
//!
//! [`TextInput`](super::TextInput)'s box, so the two cannot disagree about
//! height or border. It takes a [`Naming`] rather than a bare `ControlId`
//! because the words around it are not a label: `Every … minutes` is a
//! sentence, and its name is said for a reader with [`Naming::Hidden`].
//!
//! The value stays the text typed. Whether it is a number, and whether it is
//! at least `min`, is the caller's to say, as `invalid` and an error under it.

use leptos::prelude::*;

use super::Naming;

stylance::import_crate_style!(field, "src/kit/text_input.module.scss");
stylance::import_crate_style!(style, "src/kit/number_input.module.scss");

#[component]
pub fn NumberInput(
    /// [`Naming::Prefix`] is `Select`'s alone; here it names the field like
    /// [`Naming::Hidden`].
    naming: Naming,
    value: RwSignal<String>,
    /// The smallest value the spinner offers.
    #[prop(optional)]
    min: Option<u32>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Draws the error border and sets `aria-invalid`; the message is the
    /// caller's.
    #[prop(optional, into)]
    invalid: MaybeProp<bool>,
    /// Ids of what describes it, such as the error under it. Reactive, so it
    /// names the error only while one is drawn.
    #[prop(optional, into)]
    described_by: MaybeProp<String>,
    /// Enter, and leaving the field: where Settings saves.
    #[prop(optional)]
    on_commit: Option<Callback<()>>,
) -> impl IntoView {
    let is_invalid = Signal::derive(move || invalid.get().unwrap_or(false));
    let (id, aria_label, own) = match naming {
        Naming::FormControl(ids) => {
            let (id, by) = ids.into_attrs();
            (Some(id), None, Some(by))
        }
        Naming::Hidden(name) | Naming::Prefix(name) => (None, Some(name), None),
    };
    let described_by = move || match (own.clone(), described_by.get()) {
        (Some(own), Some(more)) => Some(format!("{own} {more}")),
        (own, more) => own.or(more),
    };
    let class = move || {
        let mut out = format!("{} {}", field::root, style::root);
        if is_invalid.get() {
            out.push(' ');
            out.push_str(field::invalid);
        }
        out
    };
    let commit = move || {
        if let Some(on_commit) = on_commit {
            on_commit.run(());
        }
    };
    view! {
        <input
            type="number"
            inputmode="numeric"
            step="1"
            min=min
            class=class
            id=id
            aria-label=aria_label
            aria-describedby=described_by
            prop:value=move || value.get()
            prop:disabled=move || disabled.get().unwrap_or(false)
            aria-invalid=move || is_invalid.get().then_some("true")
            on:input=move |ev| value.set(event_target_value(&ev))
            on:change=move |_| commit()
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn input(el: &web_sys::Element) -> web_sys::HtmlInputElement {
        el.query_selector("input")
            .unwrap()
            .expect("the field")
            .dyn_into()
            .unwrap()
    }

    /// Inside a sentence, the field's name is said rather than drawn.
    #[wasm_bindgen_test]
    fn a_hidden_name_is_the_fields_label() {
        let el = mount(|| {
            view! {
                <NumberInput
                    naming=Naming::Hidden("Minutes between checks".to_string())
                    value=RwSignal::new("1".to_string())
                    min=1
                />
            }
        });
        let field = input(&el);
        assert_eq!(
            field.get_attribute("aria-label").as_deref(),
            Some("Minutes between checks")
        );
        assert_eq!(field.get_attribute("min").as_deref(), Some("1"));
        assert_eq!(field.type_(), "number");
    }

    /// `change` is Enter or leaving the field, not each keystroke.
    #[wasm_bindgen_test]
    async fn a_change_commits_and_typing_does_not() {
        let value = RwSignal::new("1".to_string());
        let commits = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <NumberInput
                    naming=Naming::Hidden("Minutes".to_string())
                    value=value
                    on_commit=Callback::new(move |()| commits.update(|n| *n += 1))
                />
            }
        });
        let field = input(&el);
        field.set_value("5");
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
        assert_eq!(value.get_untracked(), "5");
        assert_eq!(commits.get_untracked(), 0);
        field
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
        assert_eq!(commits.get_untracked(), 1);
    }
}
