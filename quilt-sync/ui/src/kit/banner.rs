//! The outcome of something the user did.
//!
//! # A bar in the flow, not a floating toast
//!
//! It sits under the appbar and pushes the page down. A toast would need fixed
//! positioning and a stacking context, and the design bans the anchored-positioning class
//! outright — that ban is what keeps tooltip/popover/dropdown machinery out of the
//! codebase. A bar also cannot be missed by a user who happens to be looking at the
//! bottom of a long list, which is where a corner toast fails.
//!
//! # Three kinds, and the type enforces it
//!
//! `Success`, `Warning`, `Error` — the three v1 already has. A dedicated enum rather than
//! [`StateTone`](super::StateTone), which has four variants: `Neutral` has no meaning for
//! an outcome, and a type that cannot express it is better than a note asking people not
//! to. The kinds map onto tones internally, so the glyphs and colours are the page's.
//!
//! # No timer
//!
//! It does not dismiss itself. Auto-dismiss is a policy about the *operation* — a publish
//! confirmation can go quietly, an error must not — and the caller is the only thing that
//! knows which. The caller sets its signal back to `None` when it wants the bar gone.
//!
//! # It animates in and not out
//!
//! An exit animation needs the node to outlive the state that produced it: the signal goes
//! `None`, Leptos drops the view, and there is nothing left to fade. Buying that back
//! means the component owns a dismissal delay and the caller tolerates a stale node for
//! its duration — a lot of coordination for a fade nobody asked for.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/banner.module.scss");

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerVariant {
    /// It worked, and saying so is worth one line.
    Success,
    /// It worked, but something adjacent needs attention — the remote was set and its
    /// default workflow could not be resolved.
    Warning,
    /// It did not work.
    Critical,
}

impl BannerVariant {
    fn tone(self) -> StateTone {
        match self {
            Self::Success => StateTone::Success,
            Self::Warning => StateTone::Attention,
            Self::Critical => StateTone::Danger,
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Success => style::success,
            Self::Warning => style::warning,
            Self::Critical => style::critical,
        }
    }

    /// `alert` interrupts, `status` waits for a pause. An error has to cut across what is
    /// being read, because the thing the user asked for did not happen; a success does
    /// not, because it did.
    fn role(self) -> &'static str {
        match self {
            Self::Critical => "alert",
            Self::Success | Self::Warning => "status",
        }
    }
}

#[component]
pub fn Banner(
    variant: BannerVariant,
    on_dismiss: impl Fn(MouseEvent) + 'static,
    /// The message, as prose. Wraps rather than truncating — the end of an error is where
    /// the specifics are, and half an error is worse than a scrollbar.
    children: Children,
) -> impl IntoView {
    let class = format!("{} {}", style::root, variant.class());

    view! {
        <div class=class role=variant.role()>
            {variant.tone().glyph()}
            <p class=style::message>{children()}</p>
            <button
                type="button"
                class=style::dismiss
                title="Dismiss"
                aria-label="Dismiss"
                on:click=on_dismiss
            >
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
                    stroke-width="1.6" stroke-linecap="round">
                    <path d="M4.2 4.2 11.8 11.8M11.8 4.2 4.2 11.8" />
                </svg>
            </button>
        </div>
    }
}
