//! The v2 appbar's controls, shared by the pages that sit in the v2 frame.
//!
//! # Why the pages share these and the kit does not own them
//!
//! [`PageLayout`](crate::kit::PageLayout) takes its actions as a slot on purpose
//! — the appbar "has no opinion about which page needs which controls" — and
//! that is the right call: the frame should not carry page policy. What the
//! pages share is not the frame but a **reason**.
//!
//! *Settings* is on every v2 page for one fact about the experiment rather than
//! two decisions about two pages: the logo reaches `/`, and `/` renders whichever
//! main page is switched on, so it is not a way out of v2. The switch that got
//! the reader here lives in Settings, so every page behind the opt-in has to
//! offer the way back to it. That will be true of the next v2 page too.
//!
//! *Refresh* is per-page in its wiring and identical in its shape, so it takes
//! the page's own trigger and spinner.
//!
//! The gallery draws its own appbar with dead handlers, and keeps doing so: it
//! is scenery around the regions a scene demonstrates, not a region under test,
//! and borrowing this one would mean handing it a trigger it has no use for.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::kit::Button;
use crate::kit::icons;

/// The appbar's Refresh. `loading` spins it and disables it, so a second press
/// cannot send a second read.
///
/// Separate from the pages so it mounts without a Tauri host.
pub(super) fn refresh_button(reload: Trigger, refreshing: RwSignal<bool>) -> AnyView {
    view! {
        <Button
            leading_visual=icons::sync()
            loading=refreshing
            on_click=move |_| {
                refreshing.set(true);
                reload.notify();
            }
        >
            "Refresh"
        </Button>
    }
    .into_any()
}

/// Clears `refreshing` when `ready` goes true.
///
/// `ready` means no read is outstanding, never "the resource holds a value": a
/// refetching `LocalResource` keeps its previous value until the new one lands,
/// so that second question is true for the whole of a refresh.
pub(super) fn end_spin_when_ready(refreshing: RwSignal<bool>, ready: Signal<bool>) {
    Effect::new(move |_| {
        if ready.get() {
            refreshing.set(false);
        }
    });
}

/// Refresh, then Settings — the pair every v2 page carries, in that order.
///
/// Refresh first because it acts on the page you are looking at; Settings is the
/// way off it.
pub(super) fn v2_appbar_actions(reload: Trigger, refreshing: RwSignal<bool>) -> AnyView {
    let navigate = use_navigate();
    view! {
        {refresh_button(reload, refreshing)}
        <Button
            leading_visual=icons::gear()
            on_click=move |_| navigate("/settings", NavigateOptions::default())
        >
            "Settings"
        </Button>
    }
    .into_any()
}
