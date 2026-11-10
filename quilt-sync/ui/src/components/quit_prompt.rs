//! The prompt that stands between a quit and an interrupted apply.
//!
//! The backend defers the quit and raises this; the answer decides. Drawn as
//! the kit's confirmation rather than an OS dialog — see the desktop spec's
//! shared GUI chrome.

use leptos::prelude::*;
use quilt_ui_kit::QuitConfirm;

use crate::commands;
use crate::engine;

/// Event the backend raises when a quit would interrupt an apply. Kept in
/// lockstep with `tray.rs`.
const QUIT_PROMPT_EVENT: &str = "quit-prompt";

/// App-level listener. Mounted once beside the other app-level surfaces, so a
/// quit is answerable from whatever screen is open.
///
/// The desktop window's alone: quitting is the desktop app's, and one prompt
/// can't be answered in one tab and left standing in another.
#[component]
pub fn QuitPrompt() -> impl IntoView {
    let visible = RwSignal::new(false);
    if !engine::is_window() {
        return None;
    }

    let listener = engine::listen::<u64>(QUIT_PROMPT_EVENT, move |generation| {
        visible.set(true);
        // Report the prompt on screen at once. Until this lands the backend
        // treats the quit as unanswerable and lets it through after a short
        // grace period — which is the intended behavior when the window
        // cannot ask, and must not be the behavior when it can.
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::quit_prompt_shown(generation).await {
                leptos::logging::error!("quit prompt ack failed: {err}");
            }
        });
    });
    on_cleanup(move || drop(listener));

    // A failed answer leaves the engine waiting for one, so the question comes
    // back rather than leaving every later quit waiting on this one. On a
    // timer, so the dialog's own queued `close` lands first and cannot shut it
    // again.
    let ask_again = move || {
        set_timeout(
            move || {
                visible.try_set(true);
            },
            std::time::Duration::ZERO,
        );
    };
    let on_quit = move |()| {
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::quit_confirm().await {
                leptos::logging::error!("quit failed: {err}");
                ask_again();
            }
        });
    };
    let on_stay = move |()| {
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::quit_cancel().await {
                leptos::logging::error!("quit cancel failed: {err}");
                ask_again();
            }
        });
    };

    Some(view! { <QuitConfirm open=visible on_quit=on_quit on_stay=on_stay /> })
}
