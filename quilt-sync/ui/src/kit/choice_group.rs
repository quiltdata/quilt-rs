//! A named set of radios, with the consequence of the current choice under it.
//!
//! # Why a group and not a row
//!
//! Two things belong to the set rather than to any one option: the shared `name`
//! that makes the radios exclusive, and the caption. A scope and its effect —
//! *Files I pick* / *54 of 56 downloaded* — has to read as one statement, and a
//! component that only knew about one option could not say it.
//!
//! # It generates its own `name`
//!
//! Callers do not pass one, and cannot. A name that has to be unique across the
//! document is a precondition no call site can check, and the kit has already
//! been bitten: five `SegmentedControl`s sharing a literal became one radiogroup,
//! so choosing in any of them cleared the rest. **No kit component asks a caller
//! for a globally unique string.**
//!
//! # Not `ToggleRow`
//!
//! That is a checkbox row — an independent setting with two states. This is a
//! choice between named alternatives, where exactly one holds.

use leptos::prelude::*;

use super::unique_id;

stylance::import_crate_style!(style, "src/kit/choice_group.module.scss");

/// One alternative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// What the reader picks.
    pub label: String,
    /// What `selected` holds when they pick it.
    pub value: String,
}

impl Choice {
    #[must_use]
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[component]
pub fn ChoiceGroup(
    /// Names the set. Drawn, unlike `SegmentedControl`'s — the options here are
    /// alternatives whose shared subject is not obvious from reading them.
    #[prop(into)]
    label: String,
    /// What the current choice means, in the present tense. Reactive, because it
    /// changes with the choice: that is the whole reason the caption belongs to
    /// the group rather than sitting beside it as prose.
    #[prop(optional, into)]
    caption: MaybeProp<String>,
    options: Vec<Choice>,
    selected: RwSignal<String>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let name = unique_id("choice");
    let label_id = format!("{name}-label");
    let caption_id = format!("{name}-caption");
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));

    let described_by = {
        let caption_id = caption_id.clone();
        move || caption.with(|c| c.is_some().then(|| caption_id.clone()))
    };

    let rows = options
        .into_iter()
        .map(|choice| {
            let value = choice.value;
            let checked = {
                let value = value.clone();
                move || selected.with(|current| *current == value)
            };
            let pick = {
                let value = value.clone();
                move |_| selected.set(value.clone())
            };
            view! {
                <label class=style::choice>
                    <input
                        type="radio"
                        class=style::input
                        name=name.clone()
                        prop:checked=checked
                        disabled=move || is_disabled.get()
                        on:change=pick
                    />
                    <span class=style::indicator aria-hidden="true" />
                    <span>{choice.label}</span>
                </label>
            }
        })
        .collect_view();

    let class = move || {
        let mut out = String::from(style::root);
        if is_disabled.get() {
            out.push(' ');
            out.push_str(style::disabled);
        }
        out
    };

    view! {
        <div class=class role="radiogroup" aria-labelledby=label_id.clone()
            aria-describedby=described_by>
            <span class=style::label id=label_id.clone()>{label}</span>
            {rows}
            {move || {
                caption
                    .get()
                    .map(|text| {
                        view! { <p class=style::caption id=caption_id.clone()>{text}</p> }
                    })
            }}
        </div>
    }
}
