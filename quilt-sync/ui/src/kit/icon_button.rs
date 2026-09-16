//! Icon-only button.
//!
//! Separate from [`Button`](super::Button) rather than a variant of it, because the
//! job differs rather than the weight: with no label it needs an accessible name
//! of its own, it wants a square target, and it appears in list rows where a text
//! button never would.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/icon_button.module.scss");

/// Framed for chrome, Bare for row actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconButtonVariant {
    #[default]
    Default,
    Invisible,
}

#[component]
pub fn IconButton(
    /// The glyph. Sized by CSS, so callers pass an `svg` without dimensions.
    icon: AnyView,
    /// The accessible name. Goes to `aria-label` and, because we chose the `title`
    /// attribute over a tooltip component, to `title` as well — Primer splits those
    /// into a required `aria-label` plus an optional `description`, and one prop
    /// doing both jobs is named after the one that is not optional.
    ///
    /// An icon-only control without it is unusable with a screen reader and a guess
    /// with a mouse.
    #[prop(into)]
    aria_label: String,
    on_click: impl Fn(MouseEvent) + 'static,
    #[prop(optional)] variant: IconButtonVariant,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Rotates the glyph — for the appbar's refresh while a fetch is in flight.
    #[prop(optional, into)]
    spinning: MaybeProp<bool>,
    /// Whether the thing this button opens is open. An icon-only trigger with a
    /// name and nothing else tells a reader what it is called but not that it
    /// does anything, and never that it is currently showing something.
    ///
    /// Deliberately *not* paired with `aria-haspopup`: that attribute's `true`
    /// is synonymous with `menu`, which promises arrow-key navigation this kit
    /// does not implement — see [`ActionMenu`](super::ActionMenu).
    #[prop(optional, into)]
    aria_expanded: MaybeProp<bool>,
    /// The id of what it opens.
    #[prop(optional, into)]
    aria_controls: MaybeProp<String>,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));
    let is_spinning = Signal::derive(move || spinning.get().unwrap_or(false));

    let class = move || {
        let mut out = String::from(style::root);
        out.push(' ');
        out.push_str(match variant {
            IconButtonVariant::Default => style::default,
            IconButtonVariant::Invisible => style::invisible,
        });
        if is_spinning.get() {
            out.push(' ');
            out.push_str(style::spinning);
        }
        out
    };

    view! {
        <button
            type="button"
            aria-expanded=move || aria_expanded.get().map(|v| v.to_string())
            aria-controls=move || aria_controls.get()
            class=class
            aria-label=aria_label.clone()
            title=aria_label
            disabled=move || is_disabled.get()
            on:click=move |ev| {
                if !is_disabled.get() {
                    on_click(ev);
                }
            }
        >
            {icon}
        </button>
    }
}
