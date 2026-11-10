use std::future::Future;
use std::time::Duration;

use leptos::prelude::*;
use quilt_ui_kit::{UpdateBand, UpdateState};

use crate::commands;
use crate::commands::UpdateInfo;
use crate::engine;

// Intentionally short: we want to aggressively encourage updates.
// Previously this was a fully automatic updater; 5 minutes is a concession.
const DISMISS_DURATION_MS: f64 = 5.0 * 60.0 * 1000.0;
const STORAGE_KEY: &str = "updateDismissedAt";

/// How often a running app asks again. The engine reads the feed less often
/// than this and answers from its last read in between, so this mostly sets
/// how soon a dismissed band comes back.
const RECHECK: Duration = Duration::from_hours(1);

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn dismissed_at() -> Option<f64> {
    local_storage()?
        .get_item(STORAGE_KEY)
        .ok()
        .flatten()?
        .parse()
        .ok()
}

fn dismissed(at: Option<f64>, now: f64) -> bool {
    at.is_some_and(|at| now - at < DISMISS_DURATION_MS)
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

/// Checks for an update at start and every [`RECHECK`] while the app runs,
/// and hands the answer to every page. Called once, above the router, so a
/// page change never starts a second timer.
///
/// "No update" and a failed check (network error, etc.) show nothing, to
/// avoid distracting the user on every launch.
pub fn provide_update_check() {
    check_every(RECHECK, commands::check_for_update);
}

fn check_every<C, F>(every: Duration, check: C)
where
    C: Fn() -> F + 'static,
    F: Future<Output = Result<Option<UpdateInfo>, String>> + 'static,
{
    let found = RwSignal::new(None);
    provide_context(UpdateCheck { found });
    // An install running or failed is on the band; a check must not replace it.
    let busy = move || {
        found.with_untracked(|found| {
            matches!(
                found,
                Some(Found {
                    state: UpdateState::Installing | UpdateState::Failed { .. },
                    ..
                })
            )
        })
    };
    let tick = move || {
        if dismissed(dismissed_at(), js_sys::Date::now()) || busy() {
            return;
        }
        let answer = check();
        leptos::task::spawn_local(async move {
            if let Ok(Some(info)) = answer.await
                && !busy()
            {
                found.set(Some(Found {
                    state: UpdateState::Available {
                        version: info.version,
                    },
                    url: info.url,
                }));
            }
        });
    };
    tick();
    if let Ok(handle) = set_interval_with_handle(tick, every) {
        on_cleanup(move || handle.clear());
    }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use quilt_ui_kit::test_support::{mount, sleep_ms, unmount_earlier};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;
    use crate::commands::UpdateInfo;

    const MINUTE_MS: f64 = 60.0 * 1000.0;

    type Answer = std::future::Ready<Result<Option<UpdateInfo>, String>>;

    fn release() -> UpdateInfo {
        UpdateInfo {
            version: "9.9.9".to_owned(),
            url: "https://example.test/QuiltSync.dmg".to_owned(),
        }
    }

    /// A feed that offers nothing until `from` checks have been made, and
    /// counts every check.
    fn feed(from: u32) -> (Rc<Cell<u32>>, impl Fn() -> Answer + 'static) {
        let asked = Rc::new(Cell::new(0));
        let counter = asked.clone();
        let check = move || {
            counter.set(counter.get() + 1);
            std::future::ready(Ok((counter.get() > from).then(release)))
        };
        (asked, check)
    }

    fn says_update(el: &web_sys::Element) -> bool {
        el.text_content().unwrap_or_default().contains("9.9.9")
    }

    fn forget_dismiss() {
        if let Some(storage) = local_storage() {
            let _ = storage.remove_item(STORAGE_KEY);
        }
    }

    #[test]
    fn a_dismiss_lasts_five_minutes() {
        let now = 100.0 * MINUTE_MS;
        assert!(!dismissed(None, now));
        assert!(dismissed(Some(now - 4.0 * MINUTE_MS), now));
        assert!(!dismissed(Some(now - 6.0 * MINUTE_MS), now));
    }

    #[wasm_bindgen_test]
    async fn the_app_asks_again_after_the_interval() {
        forget_dismiss();
        let (asked, check) = feed(1);
        let el = mount(move || {
            check_every(Duration::from_millis(50), check);
            view! { <UpdateNotice /> }
        });
        assert_eq!(asked.get(), 1, "the first check runs at start");
        assert!(!says_update(&el), "no update at start");

        sleep_ms(120).await;
        assert!(asked.get() >= 2, "no check after the interval");
        assert!(says_update(&el), "the later check's release is not shown");
        unmount_earlier();
    }

    #[wasm_bindgen_test]
    async fn a_dismissed_band_comes_back_after_its_time() {
        // Dismissed just short of five minutes ago: the first ticks ask
        // nothing, a tick after the dismiss runs out shows the band.
        let at = js_sys::Date::now() - DISMISS_DURATION_MS + 150.0;
        local_storage()
            .unwrap()
            .set_item(STORAGE_KEY, &at.to_string())
            .unwrap();
        let (asked, check) = feed(0);
        let el = mount(move || {
            check_every(Duration::from_millis(50), check);
            view! { <UpdateNotice /> }
        });
        assert_eq!(asked.get(), 0, "a check ran within the dismiss");
        assert!(!says_update(&el));

        sleep_ms(300).await;
        assert!(asked.get() >= 1, "no check once the dismiss ran out");
        assert!(says_update(&el), "the band did not come back");
        forget_dismiss();
        unmount_earlier();
    }

    #[wasm_bindgen_test]
    fn a_page_change_starts_no_check_of_its_own() {
        forget_dismiss();
        let (asked, check) = feed(0);
        let page = RwSignal::new(true);
        mount(move || {
            check_every(Duration::from_hours(1), check);
            view! {
                <Show when=move || page.get()>
                    <UpdateNotice />
                </Show>
            }
        });
        for _ in 0..5 {
            page.set(false);
            page.set(true);
        }
        assert_eq!(asked.get(), 1, "a page's band made its own check");
        unmount_earlier();
    }
}
