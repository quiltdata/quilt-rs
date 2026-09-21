//! The v2 package page. Behind `ExperimentalSettings.package_page_v2`.
//!
//! The header is drawn; the regions below it are not. The namespace line under
//! the header is what is left of the seam's placeholder, and it stays until the
//! file pane takes that space.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::commands;
use crate::kit::{Banner, BannerVariant, LoadFailure, PageLayout};

use super::appbar::{end_spin_when_ready, v2_appbar_actions};
use super::status_watch::StatusWatch;

mod header;

use header::{PageHeader, PageHeaderSkeleton};

/// What `/installed-package` renders for a reader with *New package page* on.
#[component]
pub fn InstalledPackageV2() -> impl IntoView {
    let query = use_query_map();
    // The address is the only input the page has yet. Read reactively, because
    // one route serves every package and a link from another page swaps the
    // parameter without remounting.
    let namespace = move || query.read().get("namespace").unwrap_or_default();

    // One read for the whole page. Re-runs when the address changes, because one
    // route serves every package and a link from another page swaps the
    // parameter without remounting; and whenever `reload` fires — the watcher
    // reporting news about this package, or the failure arm's way out.
    // What the reader has already read and closed. Keyed on the message, so a
    // different pause is news again — see `pause_banner`.
    let dismissed: RwSignal<Option<String>> = RwSignal::new(None);

    let reload = Trigger::new();
    // Whether the one read is out. The main page counts, because it has four;
    // one read needs a flag.
    //
    // It ENDS the spin, and does not start it. Refresh spins only for a press,
    // here and on the main page both: `loading` implies `disabled`, so spinning
    // on a reload the watcher started would take the manual escape hatch away
    // for a reason the reader did not cause — and this button is the escape
    // hatch, the one answer to a pause whose clearing nothing announces. A
    // background refetch keeps the previous value on screen until the new one
    // lands, so nothing flickers while it runs.
    let in_flight = RwSignal::new(false);
    let data = LocalResource::new(move || {
        reload.track();
        let namespace = query.read().get("namespace").unwrap_or_default();
        async move {
            in_flight.set(true);
            let answer = commands::get_package_page_data(namespace).await;
            in_flight.set(false);
            answer
        }
    });

    let refreshing = RwSignal::new(false);
    end_spin_when_ready(refreshing, Signal::derive(move || !in_flight.get()));

    // `heading` is not reactive and one route serves every package, so it names
    // the page rather than the package; the package's own name is on screen.
    view! {
        <PackageEventListener reload=reload />
        <PageLayout
            heading="Package"
            // Its own `Suspense`, so the band can sit in the frame's slot —
            // directly under the appbar, pushing the page down — while the data
            // it needs arrives with the body's read. An empty fallback: a
            // skeleton here would reserve a band for news that usually is not
            // there, and the page would settle by collapsing it.
            banner=view! {
                <Suspense fallback=|| ()>
                    {move || Suspend::new(async move {
                        match data.await {
                            Ok(d) => pause_banner(d.sync_paused.clone(), dismissed),
                            Err(_) => ().into_any(),
                        }
                    })}
                </Suspense>
            }
                .into_any()
            actions=v2_appbar_actions(reload, refreshing)
        >
            <Suspense fallback=|| view! { <PageHeaderSkeleton /> }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => view! { <PageHeader data=d.header /> }.into_any(),
                        // The page keeps its frame and states the failure in
                        // place. A read that failed for a reason the header
                        // could have worded — no session, a refused role —
                        // never reaches here: the command resolves those to a
                        // state, and the header draws them.
                        Err(_) => {
                            view! {
                                <LoadFailure
                                    words="Could not load this package."
                                    on_retry=Callback::new(move |()| reload.notify())
                                />
                            }
                                .into_any()
                        }
                    }
                })}
            </Suspense>
            <p>{namespace}</p>
        </PageLayout>
    }
}

/// The band that says autosync has stopped, and why.
///
/// # Beside the state, not instead of it
///
/// The header says what the package needs; this says why the worker stopped.
/// Different sentences, and a package can need both at once — a newer revision
/// upstream and a workflow that rejected the last one. Folding the second into
/// the header's one label would hide the first.
///
/// Only the residue reaches here. Every other pause resolves into a state the
/// header words — a conflict names its files, a denial names the refusal — and
/// the backend sends `None` for those, so the two surfaces cannot say the same
/// thing twice.
///
/// # The words are the page's; the detail is the engine's
///
/// The lead sentence is written here, because the vocabulary is UI-owned. The
/// message is the engine's own refusal text — a workflow's complaint, a hash
/// mismatch — and nothing else knows it, so it renders as the detail after it.
///
/// # Warning, not Critical, and the two are one choice here
///
/// `BannerVariant` ties the colour to the announcement: `Critical` is
/// `role="alert"`, which interrupts, and its doc earns that by saying the thing
/// the user asked for did not happen. Nobody asked for anything here — the pause
/// was already true when the page was opened, and reading a page is not a
/// request that failed, so interrupting a screen reader on arrival would be the
/// wrong announcement. `Warning` is `role="status"`, which waits.
///
/// The colour follows the same way. `kit::PackageState::Paused` is toned Danger,
/// but that is a chip's severity while scanning many packages; the product's own
/// rendering of a pause as a standing condition — the autosync card's `Paused`
/// mark — is Attention, which is what `Warning` maps to. The header no longer
/// draws this state at all, so there is no chip on this screen for it to
/// disagree with.
///
/// # Dismissal is keyed on the message
///
/// The bar has no timer and the caller owns its dismissal. A pause is a standing
/// fact, so dismissing hides a thing that is still true — which is the reader's
/// call to make about a message they have read. But a *different* pause is news
/// again, so what is remembered is the message dismissed rather than a flag.
fn pause_banner(message: Option<String>, dismissed: RwSignal<Option<String>>) -> AnyView {
    // `StoredValue` so the derived closure is `Copy` and can be handed to both
    // the `when` and the body without cloning the message at each use.
    let message = StoredValue::new(message);
    let showing = move || {
        message
            .get_value()
            .filter(|m| dismissed.get().as_ref() != Some(m))
    };

    view! {
        <Show when=move || showing().is_some() fallback=|| ()>
            {
                let message = showing().unwrap_or_default();
                let remembered = message.clone();
                view! {
                    <Banner
                        variant=BannerVariant::Warning
                        on_dismiss=move |_| dismissed.set(Some(remembered.clone()))
                    >
                        "Autosync has stopped for this package. "
                        {message}
                    </Banner>
                }
            }
        </Show>
    }
    .into_any()
}

/// Ask the backend again when the watcher reports news about **this** package.
///
/// Renders nothing; it exists for the two subscriptions, which are dropped with
/// it. Its own component so they are registered once rather than rebuilt with a
/// payload — `main_page`'s `PackageStatusListener` for the same reason.
///
/// # Two streams, because one of them cannot see a pause
///
/// `package-status-changed` carries a fingerprint of the observation — the
/// upstream state and the changed paths — and [`StatusWatch`] drops an event
/// that repeats the last one. A pause moves neither: a workflow rejection or a
/// refused role stops syncing over a tree that has not changed, so the status
/// stream reports the same observation and the fingerprint rule correctly calls
/// it old news. `autosync-paused` is the only thing that says a pause happened,
/// so the header would never learn of one without it.
///
/// # What is still missed, and why it is not fixed here
///
/// Nothing announces a pause **clearing**. `Watcher::clear_paused` drops the
/// entry and notifies the tray, and emits no event, so the page learns a pause
/// is over only from the next status event whose fingerprint differs. For the
/// ordinary case that is enough — a pull that resolves a conflict rewrites the
/// tree, and the filesystem watcher reports it — but re-enabling autosync clears
/// every pause while moving nothing, and this page would keep the stale answer
/// until something else moved. That is an engine gap, not a page one.
///
/// # Both filters are on the namespace
///
/// One route serves every package and the watcher reports all of them, so
/// without the filter this page would refetch on every other package's news.
/// Read untracked: the listener reads the address at the moment an event
/// arrives, and must not subscribe to it.
#[component]
fn PackageEventListener(reload: Trigger) -> impl IntoView {
    let query = use_query_map();
    let watch = StatusWatch::new(reload);

    let is_ours = move |namespace: &quilt_uri::Namespace| {
        query.read_untracked().get("namespace").as_deref() == Some(namespace.to_string().as_str())
    };

    let status = crate::tauri::listen::<commands::PackageStatusEvent>(
        commands::PACKAGE_STATUS_EVENT,
        move |event| {
            if is_ours(&event.namespace) {
                watch.observe(&event);
            }
        },
    );
    let paused = crate::tauri::listen::<commands::PausedEvent>(
        commands::AUTOSYNC_PAUSED_EVENT,
        move |event| {
            if is_ours(&event.namespace) {
                // No fingerprint to compare: a pause is news the status stream
                // cannot report. The refetch re-reads the watcher's map, which
                // is authoritative, so a repeat costs one read and says the
                // same thing.
                watch.nudge();
            }
        },
    );
    on_cleanup(move || {
        drop(status);
        drop(paused);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount, sleep_ms};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use wasm_bindgen_test::*;

    /// Put the browser on an address before the router reads one. Same origin,
    /// so the history write is allowed; the runner's own page is whatever it is.
    fn go_to(address: &str) {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
    }

    /// The route parameter arrives and the page names it. A page that drew a
    /// fixed string would pass a weaker test and tell the next unit nothing.
    ///
    /// The seam's `It works!` is gone — the header took that space — and its
    /// assertion went with it. What replaced it is stronger: there is no Tauri
    /// host under the test runner, so the page's read fails, and this now also
    /// holds the failure arm to keeping the frame and the package's name rather
    /// than blanking the page.
    ///
    /// Async because the router resolves a location one tick after the mount —
    /// queried synchronously the container is still a comment marker.
    #[wasm_bindgen_test]
    async fn the_page_names_the_package_the_address_asked_for() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        element_saying(&el, "team/dataset");
        element_saying(&el, "Could not load this package.");
    }

    /// The residue's own words, and the engine's message after them. A band that
    /// only repeated the backend's sentence would be the vocabulary leaving the
    /// UI; one that dropped it would lose the only part naming what to fix.
    #[wasm_bindgen_test]
    fn the_pause_band_says_what_stopped_and_what_the_engine_reported() {
        let dismissed = RwSignal::new(None);
        let el = mount(move || {
            pause_banner(
                Some("workflow rejected the revision".to_string()),
                dismissed,
            )
        });

        let text = el.text_content().unwrap_or_default();
        assert!(
            text.contains("Autosync has stopped for this package"),
            "the page writes the sentence; markup was {}",
            el.inner_html()
        );
        assert!(
            text.contains("workflow rejected the revision"),
            "and the engine's own reason is the detail; markup was {}",
            el.inner_html()
        );
    }

    /// No pause, no band. The slot must not reserve a strip for news that is
    /// usually absent.
    #[wasm_bindgen_test]
    fn an_unpaused_package_draws_no_band() {
        let dismissed = RwSignal::new(None);
        let el = mount(move || pause_banner(None, dismissed));
        assert_eq!(el.text_content().unwrap_or_default().trim(), "");
    }

    /// Dismissal is keyed on the message, not a flag: a pause the reader has
    /// read and closed stays closed, and a different one is news again. Both
    /// halves, because a flag would pass the first and fail the second.
    #[wasm_bindgen_test]
    fn a_dismissed_pause_stays_closed_and_a_different_one_does_not() {
        let dismissed = RwSignal::new(Some("workflow rejected the revision".to_string()));

        let closed = mount(move || {
            pause_banner(
                Some("workflow rejected the revision".to_string()),
                dismissed,
            )
        });
        assert_eq!(
            closed.text_content().unwrap_or_default().trim(),
            "",
            "the message the reader closed stays closed"
        );

        let fresh = mount(move || pause_banner(Some("hash mismatch".to_string()), dismissed));
        assert!(
            fresh
                .text_content()
                .unwrap_or_default()
                .contains("hash mismatch"),
            "a different pause is news again; markup was {}",
            fresh.inner_html()
        );
    }

    /// The appbar carries the v2 pair, in order.
    ///
    /// *Refresh* matters more here than it looks: the page follows the watcher,
    /// so this is not the staleness fix it would once have been — it is the
    /// escape hatch for the one thing the streams cannot report, a pause
    /// CLEARING, which emits no event at all. A page showing the pause band can
    /// otherwise keep showing it after autosync is re-enabled.
    ///
    /// *Settings* is the way off a page the logo cannot leave: `/` renders
    /// whichever main page is switched on, so it is not an exit from v2.
    #[wasm_bindgen_test]
    async fn the_appbar_offers_refresh_and_settings() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        element_saying(&el, "Refresh");
        element_saying(&el, "Settings");
    }

    /// v2's frame, not v1's. `data-v2-page` is what the stylesheet keys the
    /// palette and `color-scheme` on, so without it the page is drawn in v1's
    /// fixed light chrome whatever the desktop is set to.
    #[wasm_bindgen_test]
    async fn the_page_is_drawn_in_the_v2_frame() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_some(),
            "the placeholder sits in the v2 page frame; markup was {}",
            el.inner_html()
        );
    }
}
