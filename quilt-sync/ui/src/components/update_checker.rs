use leptos::prelude::*;

use crate::commands;

// Intentionally short: we want to aggressively encourage updates.
// Previously this was a fully automatic updater; 5 minutes is a concession.
const DISMISS_DURATION_MS: f64 = 5.0 * 60.0 * 1000.0;
const STORAGE_KEY: &str = "updateDismissedAt";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn is_dismissed() -> bool {
    let storage = match local_storage() {
        Some(s) => s,
        None => return false,
    };
    let value = match storage.get_item(STORAGE_KEY).ok().flatten() {
        Some(v) => v,
        None => return false,
    };
    let dismissed_at: f64 = match value.parse() {
        Ok(v) => v,
        Err(_) => return false,
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

#[derive(Clone)]
enum UpdateState {
    /// No update available or still checking.
    Hidden,
    /// An update is available; display version and action buttons.
    Available(String),
    /// Download and install in progress.
    Installing,
    /// Download or install failed; show error and allow retry.
    Failed { version: String, error: String },
}

/// App-level component that checks for updates on mount and shows a
/// notification bar when one is available.
///
/// Renders its own notification UI, independent of per-page notifications.
#[component]
pub fn UpdateChecker() -> impl IntoView {
    let state = RwSignal::new(UpdateState::Hidden);

    // Check for updates on mount (unless recently dismissed).
    Effect::new(move || {
        if is_dismissed() {
            return;
        }
        leptos::task::spawn_local(async move {
            match commands::check_for_update().await {
                Ok(Some(info)) => state.set(UpdateState::Available(info.version)),
                Ok(None) => {}
                Err(_) => {
                    // Silently ignore check failures (network errors, etc.)
                    // to avoid distracting the user on every launch.
                }
            }
        });
    });

    let dismiss = move |_| {
        set_dismissed();
        state.set(UpdateState::Hidden);
    };

    let install = move |_| {
        let version = match state.get() {
            UpdateState::Available(v) | UpdateState::Failed { version: v, .. } => v,
            _ => return,
        };
        state.set(UpdateState::Installing);
        leptos::task::spawn_local(async move {
            if let Err(e) = commands::download_and_install_update().await {
                leptos::logging::error!("Update failed: {e}");
                state.set(UpdateState::Failed { version, error: e });
            }
            // On success the app restarts, so we never reach here.
        });
    };

    move || match state.get() {
        UpdateState::Hidden => None,
        UpdateState::Available(version) => Some(
            view! {
                <div class="qui-notify">
                    <div class="root">
                        <div class="update-bar">
                            <span>"Update available: " {version}</span>
                            <div class="update-bar-actions">
                                <button
                                    class="qui-button primary"
                                    type="button"
                                    on:click=install
                                >
                                    <span>"Download & Install"</span>
                                </button>
                                <button
                                    class="qui-button"
                                    type="button"
                                    on:click=dismiss
                                >
                                    <span>"Dismiss"</span>
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            }
            .into_any(),
        ),
        UpdateState::Installing => Some(
            view! {
                <div class="qui-notify">
                    <div class="root">
                        <div class="update-bar">
                            <span>"Downloading and installing update\u{2026}"</span>
                        </div>
                    </div>
                </div>
            }
            .into_any(),
        ),
        UpdateState::Failed { version, error } => Some(
            view! {
                <div class="qui-notify error">
                    <div class="root">
                        <div class="update-bar">
                            <span>"Update " {version} " failed: " {error}</span>
                            <div class="update-bar-actions">
                                <button
                                    class="qui-button primary"
                                    type="button"
                                    on:click=install
                                >
                                    <span>"Retry"</span>
                                </button>
                                <button
                                    class="qui-button"
                                    type="button"
                                    on:click=dismiss
                                >
                                    <span>"Dismiss"</span>
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            }
            .into_any(),
        ),
    }
}
