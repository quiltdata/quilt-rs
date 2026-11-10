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
/// than this and answers from its last read in between.
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

/// How much of a dismiss is left, in milliseconds; `None` once it is over.
fn dismiss_left(at: Option<f64>, now: f64, dismiss_ms: f64) -> Option<f64> {
    let left = at? + dismiss_ms - now;
    // A stored time from a changed clock can end the dismiss late, never past
    // one whole dismiss from now.
    (left.is_finite() && left > 0.0).then(|| left.min(dismiss_ms))
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
    /// While a dismiss lasts. Checks go on underneath, so the band comes back
    /// with the latest answer.
    hidden: RwSignal<bool>,
    dismiss_ms: f64,
}

impl UpdateCheck {
    fn dismiss_left(self) -> Option<f64> {
        dismiss_left(dismissed_at(), js_sys::Date::now(), self.dismiss_ms)
    }

    /// Hide the band for what is left of the last dismiss.
    fn hide_while_dismissed(self) {
        let Some(left) = self.dismiss_left() else {
            return;
        };
        self.hidden.set(true);
        set_timeout(
            move || {
                // A later dismiss has its own timer.
                if self.dismiss_left().is_none() {
                    let _ = self.hidden.try_set(false);
                }
            },
            Duration::from_secs_f64(left / 1000.0),
        );
    }
}

/// Checks for an update at start and every [`RECHECK`] while the app runs,
/// and hands the answer to every page. Called once, above the router, so a
/// page change never starts a second timer.
///
/// "No update" and a failed check (network error, etc.) show nothing, to
/// avoid distracting the user on every launch.
pub fn provide_update_check() {
    check_every(RECHECK, DISMISS_DURATION_MS, commands::check_for_update);
}

fn check_every<C, F>(every: Duration, dismiss_ms: f64, check: C)
where
    C: Fn() -> F + 'static,
    F: Future<Output = Result<Option<UpdateInfo>, String>> + 'static,
{
    let found = RwSignal::new(None);
    let update_check = UpdateCheck {
        found,
        hidden: RwSignal::new(false),
        dismiss_ms,
    };
    provide_context(update_check);
    update_check.hide_while_dismissed();
    // An install running or failed is on the band; a check must not replace
    // it. Gone with the app, an answer has nowhere to go.
    let busy = move || {
        found
            .try_with_untracked(|found| {
                matches!(
                    found,
                    Some(Found {
                        state: UpdateState::Installing | UpdateState::Failed { .. },
                        ..
                    })
                )
            })
            .unwrap_or(true)
    };
    let tick = move || {
        if busy() {
            return;
        }
        let answer = check();
        leptos::task::spawn_local(async move {
            if let Ok(Some(info)) = answer.await
                && !busy()
            {
                let _ = found.try_set(Some(Found {
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
        check.hide_while_dismissed();
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
        let hidden = check.hidden.get();
        found.get().filter(|_| !hidden).map(|Found { state, .. }| {
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

    fn dismiss(el: &web_sys::Element) {
        use wasm_bindgen::JsCast;
        el.query_selector("button[aria-label='Dismiss']")
            .unwrap()
            .expect("no dismiss on the band")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
    }

    #[test]
    fn a_dismiss_lasts_five_minutes() {
        let now = 100.0 * MINUTE_MS;
        let five = DISMISS_DURATION_MS;
        assert_eq!(dismiss_left(None, now, five), None);
        assert_eq!(
            dismiss_left(Some(now - 4.0 * MINUTE_MS), now, five),
            Some(MINUTE_MS)
        );
        assert_eq!(dismiss_left(Some(now - 6.0 * MINUTE_MS), now, five), None);
        assert_eq!(dismiss_left(Some(f64::NAN), now, five), None);
        assert_eq!(
            dismiss_left(Some(now + 60.0 * MINUTE_MS), now, five),
            Some(five),
            "a time ahead of the clock hides for one dismiss at most"
        );
    }

    #[wasm_bindgen_test]
    async fn the_app_asks_again_after_the_interval() {
        forget_dismiss();
        let (asked, check) = feed(1);
        let el = mount(move || {
            check_every(Duration::from_millis(50), DISMISS_DURATION_MS, check);
            view! { <UpdateNotice /> }
        });
        assert_eq!(asked.get(), 1, "the first check runs at start");
        assert!(!says_update(&el), "no update at start");

        sleep_ms(120).await;
        assert!(asked.get() >= 2, "no check after the interval");
        assert!(says_update(&el), "the later check's release is not shown");
        unmount_earlier();
    }

    /// The band comes back when the dismiss ends, not at the next check.
    #[wasm_bindgen_test]
    async fn a_dismissed_band_comes_back_after_its_time() {
        forget_dismiss();
        let (asked, check) = feed(0);
        let el = mount(move || {
            check_every(Duration::from_hours(1), 100.0, check);
            view! { <UpdateNotice /> }
        });
        sleep_ms(0).await;
        assert!(says_update(&el));

        dismiss(&el);
        sleep_ms(0).await;
        assert!(!says_update(&el), "the dismiss did not hide the band");

        sleep_ms(250).await;
        assert!(says_update(&el), "the band did not come back");
        assert_eq!(asked.get(), 1, "it came back on a check, not on its time");
        forget_dismiss();
        unmount_earlier();
    }

    /// A check that answers during a dismiss, one in flight when the user
    /// dismissed included, leaves the band hidden.
    #[wasm_bindgen_test]
    async fn a_check_during_a_dismiss_keeps_the_band_hidden() {
        forget_dismiss();
        let (asked, check) = feed(0);
        let el = mount(move || {
            check_every(Duration::from_millis(30), DISMISS_DURATION_MS, check);
            view! { <UpdateNotice /> }
        });
        sleep_ms(0).await;
        dismiss(&el);
        let before = asked.get();

        sleep_ms(100).await;
        assert!(asked.get() > before, "checks stopped during the dismiss");
        assert!(!says_update(&el), "a check undid the dismiss");
        forget_dismiss();
        unmount_earlier();
    }

    /// A dismiss in another window, or before a restart, still counts.
    #[wasm_bindgen_test]
    async fn an_app_opened_within_a_dismiss_waits_out_the_rest() {
        let at = js_sys::Date::now() - 100.0;
        local_storage()
            .unwrap()
            .set_item(STORAGE_KEY, &at.to_string())
            .unwrap();
        let (_, check) = feed(0);
        let el = mount(move || {
            check_every(Duration::from_hours(1), 250.0, check);
            view! { <UpdateNotice /> }
        });
        sleep_ms(0).await;
        assert!(!says_update(&el), "the band showed within the dismiss");

        sleep_ms(300).await;
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
            check_every(Duration::from_hours(1), DISMISS_DURATION_MS, check);
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
