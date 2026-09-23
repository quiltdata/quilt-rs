//! The v2 package page. Behind `ExperimentalSettings.package_page_v2`.
//!
//! The header and the first context-pane slice are drawn from one authoritative
//! read. The file pane has not landed yet, so the shell deliberately leaves its
//! growing left side empty rather than drawing provisional content.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::commands;
use crate::kit::{Banner, BannerVariant, LoadFailure, PageLayout};

use super::appbar::v2_appbar_actions;
use super::status_watch::StatusWatch;

mod bucket_form;
pub(crate) mod context_pane;
mod header;
mod role_dialog;

use context_pane::{CurrentRevisionPane, CurrentRevisionPaneSkeleton};
use header::{PageHeader, PageHeaderSkeleton};

stylance::import_crate_style!(style, "src/pages/installed_package_v2.module.scss");

/// What a command reported, and which package it reported about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub namespace: String,
    pub variant: BannerVariant,
    /// The page's own sentence. Never the backend's.
    pub lead: String,
    /// The engine's text, when it says something the lead cannot.
    pub detail: Option<String>,
}

/// The page's half of a command: what it blocks while it runs, where it
/// reports, and what to re-read when it is done.
///
/// `pub(crate)` rather than `pub(super)`: `PageHeader`'s generated props struct
/// carries it, and a prop type less visible than the props struct is what the
/// `private_interfaces` lint fires on — and warnings are denied.
#[derive(Clone, Copy)]
pub(crate) struct Wiring {
    pub busy: RwSignal<bool>,
    pub outcome: RwSignal<Option<Outcome>>,
    pub reload: Trigger,
}

/// Render one successful page payload. Kept pure so its atomic shape can be
/// tested without pretending the wasm runner has a Tauri host.
fn package_body(data: commands::PackagePageData, w: Wiring) -> AnyView {
    view! {
        <div class=style::page>
            <PageHeader data=data.header w=w />
            <div class=style::shell>
                <CurrentRevisionPane data=data.context />
            </div>
        </div>
    }
    .into_any()
}

fn package_skeleton() -> AnyView {
    view! {
        <div class=style::page>
            <PageHeaderSkeleton />
            <div class=style::shell>
                <CurrentRevisionPaneSkeleton />
            </div>
        </div>
    }
    .into_any()
}

fn package_failure(namespace: String, reload: Trigger) -> AnyView {
    view! {
        <div class=style::page>
            <h2 class=style::identity>{namespace}</h2>
            <LoadFailure
                words="Could not load this package."
                on_retry=Callback::new(move |()| reload.notify())
            />
        </div>
    }
    .into_any()
}

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
    // One command at a time. Every control on the header reads this, so a second
    // cannot start on top of the first — two writes to one working tree is a race
    // the page has no way to arbitrate.
    let busy = RwSignal::new(false);
    // What the last command said, and which package it said it about. Keyed,
    // because a result arriving for a package the page no longer shows is not
    // this page's news — see `outcome_band`.
    let outcome: RwSignal<Option<Outcome>> = RwSignal::new(None);

    let reload = Trigger::new();
    // Whether the one read is out. The main page counts, because it has four;
    // one read needs a flag.
    //
    // It drives Refresh's spinner, so the button reports a read the watcher
    // started as readily as one the reader asked for — the page is working
    // either way, and a button that only knows about presses says nothing while
    // the page refetches under it.
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
                {outcome_band(outcome, Signal::derive(namespace))}
                <Suspense fallback=|| ()>
                    {move || Suspend::new(async move {
                        match data.await {
                            Ok(d) => pause_banner(d.sync_paused.clone(), dismissed),
                            // A failed read still says nothing about a command that ran
                            // before it; the outcome band above is outside this Suspense
                            // for exactly that reason.
                            Err(_) => ().into_any(),
                        }
                    })}
                </Suspense>
            }
                .into_any()
            actions=v2_appbar_actions(reload, in_flight.into())
        >
            <Suspense fallback=package_skeleton>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => package_body(d, Wiring { busy, outcome, reload }),
                        // The page keeps its frame and states the failure in
                        // place. A read that failed for a reason the header
                        // could have worded — no session, a refused role —
                        // never reaches here: the command resolves those to a
                        // state, and the header draws them.
                        Err(_) => package_failure(namespace(), reload),
                    }
                })}
            </Suspense>
        </PageLayout>
    }
}

/// What the last command said — the remainder channel.
///
/// # It carries only what no other surface says
///
/// A navigation reports by arriving, a dialog holds its own refusal, and a
/// pull posts its report to the notification stack. What is left for this band
/// is a non-dialog command's failure, an undo that succeeded (the state label
/// can read the same before and after, so the re-read is not a report), and a
/// remote set whose workflow could not be resolved.
///
/// # Keyed to the package, and dropped whole when it does not match
///
/// One route serves every package, so a command's result can arrive after the
/// reader has moved to another one. It is discarded rather than drawn: a
/// reader cannot tell a stale outcome from a fresh one by its text. Same rule
/// the form dialog applies to a stale session.
///
/// # The lead is the page's and the detail is the engine's
///
/// The split the pause band already makes. The vocabulary is UI-owned, so the
/// sentence saying what did not happen is written here; the engine's own
/// refusal text is the part nothing else knows, and follows as the detail.
fn outcome_band(outcome: RwSignal<Option<Outcome>>, showing: Signal<String>) -> AnyView {
    let mine = move || outcome.get().filter(|o| o.namespace == showing.get());
    view! {
        <Show when=move || mine().is_some() fallback=|| ()>
            {
                let said = mine().expect("checked by the guard above");
                view! {
                    <Banner
                        variant=said.variant
                        on_dismiss=move |_| outcome.set(None)
                    >
                        {said.lead}
                        {said.detail.map(|detail| view! { " " {detail} })}
                    </Banner>
                }
            }
        </Show>
    }
    .into_any()
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
/// # Critical, because a stopped sync is a failure
///
/// `DESIGN.md` requires a band and a chip to agree — *"a warning on the page and
/// a warning on a row cannot disagree about what amber means"* — and the kit
/// tones `PackageState::Paused` Danger. Amber here said the opposite of red
/// there about one fact.
///
/// The cost is taken deliberately rather than worked around. `BannerVariant`
/// welds colour to announcement, so `Critical` is also `role="alert"`, which
/// interrupts a reader on arrival for something that was already true before
/// they opened the page. That is the wrong shape of announcement and the right
/// colour, and the colour wins: autosync having stopped is a failure, and a band
/// that says so quietly in amber understates it.
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
                        variant=BannerVariant::Critical
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

    fn page_data() -> commands::PackagePageData {
        commands::PackagePageData {
            header: commands::PackageHeaderData {
                namespace: "team/dataset".try_into().unwrap(),
                uri: None,
                state: crate::kit::PackageState::Latest,
                remote_locked: false,
                has_local_commit: false,
                commit_has_parent: false,
                role_switch: None,
            },
            context: commands::PackageContextData {
                revision: commands::CurrentRevisionData {
                    message: Some("Initial upload".to_string()),
                    obtained_at: 1_758_500_000_000.0,
                },
                bucket: Some("quilt-lab-plates".to_string()),
            },
            sync_paused: None,
        }
    }

    fn said(namespace: &str, variant: BannerVariant, lead: &str, detail: Option<&str>) -> Outcome {
        Outcome {
            namespace: namespace.to_string(),
            variant,
            lead: lead.to_string(),
            detail: detail.map(ToString::to_string),
        }
    }

    /// The band is the surface's sentence first and the engine's text after it —
    /// the same split the pause band makes. A band that only repeated the
    /// backend would be the vocabulary leaving the UI; one that dropped it would
    /// lose the only part naming what went wrong.
    #[wasm_bindgen_test]
    fn the_outcome_band_leads_with_the_page_s_sentence() {
        let outcome = RwSignal::new(Some(said(
            "team/dataset",
            BannerVariant::Critical,
            "Could not get the latest revision.",
            Some("Failed to pull package: connection reset"),
        )));
        let el =
            mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));

        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("Could not get the latest revision."));
        assert!(
            text.contains("connection reset"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// The keying. A result arriving for a package the page no longer shows is
    /// dropped WHOLE — not greyed, not queued. A reader cannot tell a stale
    /// outcome from a fresh one by its text, which is the defect quilt-rs#974's
    /// review found.
    #[wasm_bindgen_test]
    fn an_outcome_for_another_package_is_dropped_whole() {
        let outcome = RwSignal::new(Some(said(
            "team/other",
            BannerVariant::Success,
            "The last revision was undone.",
            None,
        )));
        let el =
            mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));

        assert_eq!(el.text_content().unwrap_or_default().trim(), "");
    }

    /// Both bands at once. A pause is a standing condition and an outcome is
    /// what just happened; a package can be both, and neither replaces the other.
    #[wasm_bindgen_test]
    fn an_outcome_and_a_pause_stack_rather_than_replacing_each_other() {
        let outcome = RwSignal::new(Some(said(
            "team/dataset",
            BannerVariant::Success,
            "The last revision was undone.",
            None,
        )));
        let dismissed = RwSignal::new(None);
        let el = mount(move || {
            view! {
                {outcome_band(outcome, Signal::derive(|| "team/dataset".to_string()))}
                {pause_banner(Some("workflow rejected the revision".to_string()), dismissed)}
            }
        });

        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("The last revision was undone."));
        assert!(text.contains("Autosync has stopped for this package"));
    }

    /// A success waits for a pause in the reader's work; a failure cuts across
    /// it. That is `BannerVariant`'s own rule and the band must not quietly
    /// invert it.
    #[wasm_bindgen_test]
    fn a_failure_interrupts_and_a_success_does_not() {
        for (variant, role) in [
            (BannerVariant::Critical, "alert"),
            (BannerVariant::Success, "status"),
        ] {
            let outcome = RwSignal::new(Some(said("team/dataset", variant, "Something.", None)));
            let el =
                mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));
            let band = el.query_selector("[role]").unwrap().expect("a band");
            assert_eq!(band.get_attribute("role").as_deref(), Some(role));
        }
    }

    /// A successful payload swaps the header and pane together. The old loose
    /// paragraph was only scaffolding; package identity now belongs to the
    /// header while the pane is a named complementary landmark.
    #[wasm_bindgen_test]
    fn a_successful_payload_draws_the_real_body_without_the_placeholder() {
        // Inside a `Router`, because the header asks for a navigator.
        let el = mount(|| {
            let w = Wiring {
                busy: RwSignal::new(false),
                outcome: RwSignal::new(None),
                reload: Trigger::new(),
            };
            view! { <Router>{package_body(page_data(), w)}</Router> }
        });
        let aside = el
            .query_selector("aside")
            .unwrap()
            .expect("the context pane");
        assert_eq!(
            aside.get_attribute("aria-label").as_deref(),
            Some("About this package")
        );
        assert!(
            el.query_selector("p").unwrap().is_none(),
            "the loose namespace placeholder is gone; markup was {}",
            el.inner_html()
        );
    }

    /// The pane's fixed measure is a wide-layout decision, and the page's own
    /// inline container releases it when that shell narrows.
    #[test]
    fn the_shell_and_pane_styles_own_the_responsive_width() {
        const PAGE: &str = include_str!("installed_package_v2.module.scss");
        const PANE: &str = include_str!("installed_package_v2/context_pane.module.scss");

        assert!(PAGE.contains("container-type: inline-size"));
        assert!(PAGE.contains("justify-content: flex-end"));
        assert!(PANE.contains("width: 280px"));
        assert!(PANE.contains("@container (max-width: 800px)"));
        assert!(PANE.contains("width: 100%"));
        assert!(
            !PANE.contains('#'),
            "the slice introduces no literal colour"
        );
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

    /// The band agrees with the chip, which `DESIGN.md` requires of every tone
    /// and this one got wrong: `PackageState::Paused` is Danger, so the band is
    /// `Critical`. Asserted through `role`, which is what the variant produces
    /// — a test on the enum would restate the call site.
    #[wasm_bindgen_test]
    fn the_pause_band_is_toned_as_the_failure_it_reports() {
        let dismissed = RwSignal::new(None);
        let el = mount(move || {
            pause_banner(
                Some("workflow rejected the revision".to_string()),
                dismissed,
            )
        });

        let band = el
            .query_selector("[role]")
            .unwrap()
            .expect("the band carries a role");
        assert_eq!(
            band.get_attribute("role").as_deref(),
            Some("alert"),
            "markup was {}",
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
