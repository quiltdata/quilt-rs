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

/// The appbar's Refresh.
///
/// `busy` is the page's own answer to *is a read out* — a count on the main
/// page, a flag on the package page — and it drives `loading`, which spins the
/// button and blocks a second press.
///
/// **Read, not owned.** The button used to raise a signal of its own on click
/// and have an effect lower it when the page went quiet, which meant it reported
/// only the presses it had seen: a reload the watcher started refetched the page
/// under a button that said nothing was happening. Reading the page's own
/// in-flight state instead makes it honest whoever asked, and deletes the effect
/// that lowered it — a read either is out or is not, and the page already knows.
///
/// Separate from the pages so it mounts without a Tauri host.
pub(super) fn refresh_button(reload: Trigger, busy: Signal<bool>) -> AnyView {
    view! {
        <Button leading_visual=icons::sync() loading=busy on_click=move |_| reload.notify()>
            "Refresh"
        </Button>
    }
    .into_any()
}

/// Refresh, then Settings — the pair every v2 page carries, in that order.
///
/// Refresh first because it acts on the page you are looking at; Settings is the
/// way off it.
pub(super) fn v2_appbar_actions(reload: Trigger, busy: Signal<bool>) -> AnyView {
    let navigate = use_navigate();
    view! {
        {refresh_button(reload, busy)}
        <Button
            leading_visual=icons::gear()
            on_click=move |_| navigate("/settings", NavigateOptions::default())
        >
            "Settings"
        </Button>
    }
    .into_any()
}
