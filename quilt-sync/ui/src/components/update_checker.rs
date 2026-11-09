use leptos::prelude::*;
use quilt_ui_kit::{UpdateBand, UpdateState};

use crate::commands;
use crate::engine;

// Intentionally short: we want to aggressively encourage updates.
// Previously this was a fully automatic updater; 5 minutes is a concession.
const DISMISS_DURATION_MS: f64 = 5.0 * 60.0 * 1000.0;
const STORAGE_KEY: &str = "updateDismissedAt";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn is_dismissed() -> bool {
    let Some(storage) = local_storage() else {
        return false;
    };
    let Some(value) = storage.get_item(STORAGE_KEY).ok().flatten() else {
        return false;
    };
    let Ok(dismissed_at) = value.parse::<f64>() else {
        return false;
    };
    let now = js_sys::Date::now();
    now - dismissed_at < DISMISS_DURATION_MS
}

fn set_dismissed() {
    if let Some(storage) = local_storage() {
        let now = js_sys::Date::now().to_string();
        let _ = storage.set_item(STORAGE_KEY, &now);
    }
}

/// What the check found, and where a release is downloaded from.
#[derive(Clone)]
struct Found {
    state: UpdateState,
    url: String,
}

/// The app's one update check, shared by every page's band row.
#[derive(Clone, Copy)]
pub struct UpdateCheck {
    found: RwSignal<Option<Found>>,
}

/// Checks for an update once, unless one was dismissed recently, and hands
/// the answer to every page. Called once, above the router.
///
/// "No update" and a failed check (network error, etc.) show nothing, to
/// avoid distracting the user on every launch.
pub fn provide_update_check() {
    let found = RwSignal::new(None);
    provide_context(UpdateCheck { found });
    if is_dismissed() {
        return;
    }
    leptos::task::spawn_local(async move {
        if let Ok(Some(info)) = commands::check_for_update().await {
            found.set(Some(Found {
                state: UpdateState::Available {
                    version: info.version,
                },
                url: info.url,
            }));
        }
    });
}

/// The update band, in a page's band row; nothing while there is no update.
#[component]
pub fn UpdateNotice() -> impl IntoView {
    let check = use_context::<UpdateCheck>()?;
    let found = check.found;

    let snooze = move |()| {
        set_dismissed();
        found.set(None);
    };

    // Install, where the engine can; headless can't install itself, so the
    // user downloads it and restarts QuiltSync.
    let install = move |()| {
        let Some(Found { state, url }) = found.get_untracked() else {
            return;
        };
        if engine::headless().get_untracked() {
            if let Some(window) = web_sys::window() {
                drop(window.open_with_url_and_target_and_features(&url, "_blank", "noopener"));
            }
            return;
        }
        let (UpdateState::Available { version } | UpdateState::Failed { version, .. }) = state
        else {
            return;
        };
        found.set(Some(Found {
            state: UpdateState::Installing,
            url: url.clone(),
        }));
        leptos::task::spawn_local(async move {
            if let Err(error) = commands::download_and_install_update().await {
                leptos::logging::error!("Update failed: {error}");
                found.set(Some(Found {
                    state: UpdateState::Failed { version, error },
                    url,
                }));
            }
            // On success the app restarts, so we never reach here.
        });
    };

    let install_label =
        Signal::derive(move || engine::headless().get().then(|| "Download".to_string()));

    Some(move || {
        found.get().map(|Found { state, .. }| {
            view! {
                <UpdateBand
                    state=state
                    on_install=install
                    on_snooze=snooze
                    install_label=install_label
                />
            }
        })
    })
}
