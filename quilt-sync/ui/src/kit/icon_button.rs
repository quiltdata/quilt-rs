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
    Framed,
    Bare,
}

#[component]
pub fn IconButton(
    /// The glyph. Sized by CSS, so callers pass an `svg` without dimensions.
    icon: AnyView,
    /// Goes to both `aria-label` and `title`. An icon-only control without one is
    /// unusable with a screen reader and a guess with a mouse.
    #[prop(into)]
    label: String,
    on_click: impl Fn(MouseEvent) + 'static,
    #[prop(optional)] variant: IconButtonVariant,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Rotates the glyph — for the appbar's refresh while a fetch is in flight.
    #[prop(optional, into)]
    spinning: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));
    let is_spinning = Signal::derive(move || spinning.get().unwrap_or(false));

    let class = move || {
        let mut out = String::from(style::root);
        out.push(' ');
        out.push_str(match variant {
            IconButtonVariant::Framed => style::framed,
            IconButtonVariant::Bare => style::bare,
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
            class=class
            aria-label=label.clone()
            title=label
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
