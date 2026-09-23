//! Refresh and Settings: the controls on every redesigned appbar, v1 routes
//! included.
//!
//! Refresh takes its behaviour from the caller: a v2 page passes its refetch and
//! in-flight state, and the v1 `Layout` passes a window reload — or, on a transit
//! screen, draws it disabled.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::kit::Button;
use crate::kit::icons;

/// `busy` is the page's own in-flight state, not the button's, so a read the
/// page started without a press still shows.
pub fn refresh_button(refresh: impl Fn() + 'static, busy: Signal<bool>) -> AnyView {
    view! {
        <Button leading_visual=icons::sync() loading=busy on_click=move |_| refresh()>
            "Refresh"
        </Button>
    }
    .into_any()
}

pub fn appbar_actions(refresh: impl Fn() + 'static, busy: Signal<bool>) -> AnyView {
    with_settings(refresh_button(refresh, busy), false)
}

/// For a screen whose mount is its operation, where a reload would repeat it,
/// and so would Back onto it: Settings replaces the screen's history entry.
pub fn transit_appbar_actions() -> AnyView {
    with_settings(
        view! {
            <Button leading_visual=icons::sync() disabled=true on_click=|_| {}>
                "Refresh"
            </Button>
        }
        .into_any(),
        true,
    )
}

fn with_settings(refresh: AnyView, replace: bool) -> AnyView {
    let navigate = use_navigate();
    view! {
        {refresh}
        <Button
            leading_visual=icons::gear()
            on_click=move |_| {
                navigate(
                    "/settings",
                    NavigateOptions {
                        replace,
                        ..NavigateOptions::default()
                    },
                );
            }
        >
            "Settings"
        </Button>
    }
    .into_any()
}
