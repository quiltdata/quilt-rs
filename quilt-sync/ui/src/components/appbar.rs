//! The redesigned appbar's controls, for every route that draws that bar.
//!
//! # Why they are shared, and why here
//!
//! [`Appbar`](crate::kit::Appbar) takes its controls as a slot on purpose — the
//! bar "has no opinion about which page needs which controls" — and that is the
//! right call: the kit should not carry page policy. What the routes share is
//! not the bar but the **pair on it**. Every appbar has always carried Refresh
//! and Settings — v1's has both, and so did the v2 pages' — so the redesigned bar
//! carries the same two wherever it replaces one, and *New design preview*
//! changes what they look like, not whether they are there.
//!
//! Here rather than beside the pages, because the v1 shell draws them too:
//! [`Layout`](super::Layout) puts the redesigned bar on every v1 route that
//! draws an appbar while the preview is on, and it cannot reach into `pages`.
//!
//! # What Refresh does is the caller's
//!
//! The pair's shape is shared; Refresh's behaviour is not. A v2 page hands it the
//! page's own refetch and its in-flight state. The v1 shell hands it the
//! whole-window reload v1's own Refresh has always been, with nothing in flight
//! to report — adopting the redesigned bar does not make a v1 page adopt the v2
//! data lifecycle with it.
//!
//! That reload is also why the shell does not draw this on a transit screen: see
//! [`Layout`](super::Layout)'s `transit`.
//!
//! The gallery draws its own controls with dead handlers, and keeps doing so: the
//! bar is scenery around the regions a scene demonstrates, not a region under
//! test, and borrowing these would mean handing it a refresh it has no use for.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::kit::Button;
use crate::kit::icons;

/// The appbar's Refresh.
///
/// `refresh` is what a press does — the page's own. `busy` is the page's own
/// answer to *is a read out* — a count on the main page, a flag on the package
/// page, never on v1 — and it drives `loading`, which spins the button and
/// blocks a second press.
///
/// **Read, not owned.** The button used to raise a signal of its own on click
/// and have an effect lower it when the page went quiet, which meant it reported
/// only the presses it had seen: a reload the watcher started refetched the page
/// under a button that said nothing was happening. Reading the page's own
/// in-flight state instead makes it honest whoever asked, and deletes the effect
/// that lowered it — a read either is out or is not, and the page already knows.
///
/// Separate from the pages so it mounts without a Tauri host.
pub fn refresh_button(refresh: impl Fn() + 'static, busy: Signal<bool>) -> AnyView {
    view! {
        <Button leading_visual=icons::sync() loading=busy on_click=move |_| refresh()>
            "Refresh"
        </Button>
    }
    .into_any()
}

/// Refresh, then Settings — the pair every redesigned appbar carries, in that
/// order.
///
/// Refresh first because it acts on the page you are looking at; Settings is the
/// way off it. It is on every route that draws the bar because the switch that
/// put the reader in the redesign lives in Settings, so wherever the redesign
/// shows, the way back to that switch has to show with it.
pub fn appbar_actions(refresh: impl Fn() + 'static, busy: Signal<bool>) -> AnyView {
    let navigate = use_navigate();
    view! {
        {refresh_button(refresh, busy)}
        <Button
            leading_visual=icons::gear()
            on_click=move |_| navigate("/settings", NavigateOptions::default())
        >
            "Settings"
        </Button>
    }
    .into_any()
}
