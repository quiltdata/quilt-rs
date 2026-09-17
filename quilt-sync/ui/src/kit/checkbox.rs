//! A tri-state checkbox: the kit's only one.
//!
//! # Why the state is an enum
//!
//! The DOM models the third state as a flag *beside* `checked`, so the two can
//! disagree: `checked = true, indeterminate = true` renders mixed and ignores the
//! tick. A caller that believed it had set "all selected" would be looking at
//! "some selected" with nothing to tell it apart. [`CheckState`] makes that
//! unrepresentable — there is one value, and it has three cases.
//!
//! # Why it wraps a real `<input>`
//!
//! DESIGN.md's **Platform Owns The Keyboard Rule** names the native checkbox
//! specifically. The input is off-screen rather than replaced, exactly as
//! [`ToggleRow`](super::ToggleRow) had it first: space still toggles, the label
//! association still works, and the accessibility tree gets a checkbox rather
//! than a `div` claiming to be one.
//!
//! # It never draws its own text
//!
//! Callers own the label, because they disagree about what the label is. A
//! [`ToggleRow`](super::ToggleRow) wraps box and text in one `<label>`;
//! [`EntryGroup`](super::EntryGroup) cannot, because its heading also holds a
//! disclosure `<button>` and a `<label>` may not contain another labelable
//! element. That is why `aria_label` exists, and why it is not an edge case.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/checkbox.module.scss");

/// How the box draws, as one value rather than two booleans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    /// Nothing selected.
    #[default]
    Off,
    /// Everything selected.
    On,
    /// Some but not all — a group with three of its twelve ticked. Never
    /// reachable by clicking: a click always resolves to `On` or `Off`.
    Mixed,
}

impl From<bool> for CheckState {
    fn from(checked: bool) -> Self {
        if checked { Self::On } else { Self::Off }
    }
}

impl CheckState {
    /// What a click means. `Mixed` selects the rest rather than clearing, which
    /// is what `3 of 17 selected` → `17 of 17` reads as, and what every desktop
    /// file manager does.
    #[must_use]
    pub fn toggled(self) -> bool {
        !matches!(self, Self::On)
    }
}

#[component]
pub fn Checkbox(
    #[prop(into)] state: Signal<CheckState>,
    /// The value a click asks for — never the current one. Handed
    /// [`CheckState::toggled`] so the rule lives in one place.
    on_toggle: impl Fn(bool) + 'static,
    /// Needed only when no ancestor `<label>` names the box. Inside a
    /// `<label>` it must stay unset: a second name wins over the wrapping one,
    /// and the row's own words are the better name.
    #[prop(optional, into)]
    aria_label: MaybeProp<String>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));

    view! {
        <span class=style::root>
            <input
                type="checkbox"
                class=style::input
                prop:checked=move || matches!(state.get(), CheckState::On)
                // A property, not an attribute: there is no `indeterminate`
                // attribute to set, and writing one does nothing at all.
                prop:indeterminate=move || matches!(state.get(), CheckState::Mixed)
                disabled=move || is_disabled.get()
                aria-label=move || aria_label.get()
                on:change=move |_| on_toggle(state.get().toggled())
            />
            // `data-` and not a class: the attribute survives stylance's hashing,
            // so a parent such as `ToggleRow` can give the box its hover without
            // reaching into this module's generated class names.
            <span class=style::indicator data-q-checkbox aria-hidden="true">
                <svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2"
                    stroke-linecap="round" stroke-linejoin="round">
                    <path class=style::tick d="M2.5 6.25 4.75 8.5 9.5 3.75" />
                    <path class=style::dash d="M3 6h6" />
                </svg>
            </span>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This component's own stylesheet. The rules below are about layout, and
    /// the wasm harness mounts no stylesheet at all — `element_from_point` in a
    /// document with no CSS reports nothing, so the only place this is checkable
    /// is the source. `sync_scope.rs` reads its own file for the same reason.
    const STYLES: &str = include_str!("checkbox.module.scss");

    /// The `.input` rule, without the rules that merely mention it.
    fn input_rule() -> &'static str {
        let start = STYLES.find("\n.input {").expect("an `.input` rule") + "\n.input {".len();
        let end = start + STYLES[start..].find('}').expect("its closing brace");
        &STYLES[start..end]
    }

    /// **A mouse aimed at the box must reach the input.**
    ///
    /// This shipped broken once. The input was parked off-screen, so the drawn
    /// box was only clickable through a wrapping `<label>` — and the two callers
    /// that cannot have one, a group heading holding a `<button>` and any
    /// standalone box, took focus and ignored the mouse. It survived a browser
    /// check because that check dispatched a click *at the input*, which passes
    /// whether or not the input can be hit.
    #[test]
    fn the_input_covers_the_box_it_draws() {
        let rule = input_rule();
        for property in ["position: absolute", "inset: 0", "opacity: 0"] {
            assert!(
                rule.contains(property),
                "`.input` must carry `{property}` — without it the drawn box is \
                 paint the pointer passes straight through:\n{rule}",
            );
        }
    }

    /// The specific shape of the bug, named so nobody reintroduces it by
    /// reaching for the usual visually-hidden recipe.
    #[test]
    fn the_input_is_not_hidden_off_screen() {
        let rule = input_rule();
        assert!(
            !rule.contains("clip-path"),
            "a clipped input is unreachable by pointer; it must be transparent \
             and in place instead:\n{rule}",
        );
    }

    /// A click never asks for `Mixed`, and asks for `true` from both of the
    /// states that are not `On`.
    #[test]
    fn a_click_resolves_mixed_upwards() {
        assert!(CheckState::Off.toggled());
        assert!(CheckState::Mixed.toggled());
        assert!(!CheckState::On.toggled());
    }

    #[test]
    fn a_bool_is_never_mixed() {
        assert_eq!(CheckState::from(true), CheckState::On);
        assert_eq!(CheckState::from(false), CheckState::Off);
    }
}
