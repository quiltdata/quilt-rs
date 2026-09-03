//! The v2 main page. Behind `ExperimentalSettings.main_page_v2`.
//!
//! One region so far. `Transition`, never `Suspense` (§6): a later plan will wire a
//! refetch on every autosync transition, publish and pause, and a `Suspense`
//! boundary re-shows its fallback each time, so the page would strobe. Today
//! `reload` fires only from the Refresh button.

mod autosync;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::commands::MainPagePackageRefreshData;

/// One row's data, as the light phase delivered it: namespace, state, whether the
/// state is still the light phase's guess, when the copy last changed, and the host
/// whose role selector its switch affordance would open. A tuple rather than a
/// struct because it is local to this file and never crosses a boundary.
type PackageRowData = (String, PackageState, bool, Option<f64>, Option<String>);

/// Copied from the gallery's own helpers rather than shared: the gallery modules are
/// not compiled into the app binary, and the kit deliberately owns no icons — a caller
/// passes the glyph, so the appbar's owner draws it.
fn gear_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4">
            <circle cx="8" cy="8" r="2.1" />
            <path d="M8 1.6v1.7M8 12.7v1.7M2.5 8H4.2M11.8 8h1.7M4.1 4.1l1.2 1.2M10.7 10.7l1.2 1.2M11.9 4.1l-1.2 1.2M5.3 10.7l-1.2 1.2" />
        </svg>
    }
    .into_any()
}

fn refresh_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4" stroke-linecap="round">
            <path d="M13.5 8a5.5 5.5 0 1 1-1.9-4.15" />
            <path d="M13.6 1.9v2.4h-2.4" />
        </svg>
    }
    .into_any()
}
use crate::kit::Card;
use crate::kit::IconButton;
use crate::kit::PackageRow;
use crate::kit::PackageRowSkeleton;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::kit::Site;
use crate::kit::render;

/// The fixed sentence shown when the fetch fails. The backend's error text is
/// logged (see `render_fetch_error`) but never shown: §5's words-come-from-`render`
/// constraint applies to this file too, and a raw `Result<_, String>` error is not
/// a word from the vocabulary.
const FETCH_ERROR_WORDS: &str = "Could not load your packages.";

/// The failure branch, split out from `MainPage` so it can be tested without a
/// Tauri host. Renders only the fixed sentence for the user — logging the
/// backend's error for a developer happens once, where the fetch result is
/// handled (`MainPage`'s `Err` arm below), not here: this is a view function,
/// and view functions can re-run on every re-render, which would re-log the
/// same failure each time.
fn render_fetch_error() -> impl IntoView {
    view! { <p>{FETCH_ERROR_WORDS}</p> }
}

/// A row's live state: the light phase's guess, replaced in place by the heavy
/// phase's answer.
#[derive(Clone, Copy)]
struct RowSignals {
    state: RwSignal<PackageState>,
    provisional: RwSignal<bool>,
    role_switch_host: RwSignal<Option<String>>,
}

impl RowSignals {
    fn new(state: PackageState, role_switch_host: Option<String>) -> Self {
        Self {
            state: RwSignal::new(state),
            provisional: RwSignal::new(true),
            role_switch_host: RwSignal::new(role_switch_host),
        }
    }

    /// Replace the light phase's guess with the heavy phase's answer — in BOTH
    /// directions. The pre-filter that seeds these is an optimistic hint, and
    /// only ever adding a mark made a false positive permanent for the life of
    /// the page. The refresh is the real call, so it gets the last word.
    fn apply(self, refreshed: MainPagePackageRefreshData) {
        self.state.set(refreshed.state);
        self.role_switch_host.set(refreshed.role_switch_host);
        self.provisional.set(false);
    }
}

/// One row, with its own heavy-phase refresh. Firing one `refresh_main_page_package`
/// call per row, rather than a single call for the whole page, is what makes a row
/// settle where it stands instead of the page clearing all at once (Ruling R0).
#[component]
fn PackageListRow(
    namespace: String,
    initial_state: PackageState,
    changed_at: Option<f64>,
    role_switch_host: Option<String>,
) -> impl IntoView {
    let row = RowSignals::new(initial_state, role_switch_host);

    // One invocation per row, concurrently. The two shared resources behind it —
    // credential vending and the `/me` role query — are already serialised inside
    // the backend, and a serial walk would make the list as slow as its slowest
    // package while clearing every row at once, which is the spinner §7 rejected.
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_flag = cancelled.clone();
    on_cleanup(move || cancelled.store(true, Ordering::Relaxed));

    let ns = namespace.clone();
    leptos::task::spawn_local(async move {
        let result = commands::refresh_main_page_package(ns).await;
        if cancelled_flag.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(refreshed) => row.apply(refreshed),
            Err(err) => {
                // The row keeps the light phase's state and stays provisional,
                // which is honest: nothing confirmed it. The error is logged, not
                // rendered — the words a user reads come only from `kit::render`.
                web_sys::console::error_1(
                    &format!("refresh_main_page_package failed: {err}").into(),
                );
            }
        }
    });

    // The namespace has to travel in the query string: the package page reads it
    // with `use_query_map`, and a bare path leaves it empty.
    let href = format!("/installed-package?namespace={namespace}&filter=unmodified");
    let words = Signal::derive(move || render(&row.state.get(), Site::ListRow).words);
    let tone = Signal::derive(move || render(&row.state.get(), Site::ListRow).tone);

    view! {
        <PackageRow
            namespace=namespace
            href=href
            changed_at=changed_at
            state=words
            tone=tone
            provisional=row.provisional
        />
    }
}

/// The rows, as direct children of the card body — no wrapper element. The
/// card's own `.body > * + *` rule (`kit/card.module.scss`) spaces them; a
/// wrapping `div` would defeat that direct-child selector, and gallery
/// chrome's `.g-rows` is not shipped to the app bundle at all (`app.scss`
/// excludes it). Split out from `MainPage` so it can be tested without a
/// Tauri host.
#[component]
fn PackageList(packages: Vec<PackageRowData>) -> impl IntoView {
    packages
        .into_iter()
        // `provisional` is the light phase's own statement about the payload — every
        // row starts provisional by construction now, so the row itself has nothing
        // left to read here. It stays on the wire and in this tuple because a later
        // plan's attention queue reads it to decide what may not be listed yet.
        .map(
            |(namespace, state, _provisional, changed_at, role_switch_host)| {
                view! {
                    <PackageListRow
                        namespace=namespace
                        initial_state=state
                        changed_at=changed_at
                        role_switch_host=role_switch_host
                    />
                }
            },
        )
        .collect_view()
}

#[component]
pub fn MainPage() -> impl IntoView {
    let reload = Trigger::new();
    let packages = LocalResource::new(move || {
        reload.track();
        async move { commands::get_main_page_packages().await }
    });
    let navigate = use_navigate();

    view! {
        <PageLayout actions=view! {
            <IconButton
                icon=refresh_icon()
                aria_label="Refresh"
                on_click=move |_| reload.notify()
            />
            // The only way back to Settings from here. `/` redirects straight back to
            // this page while the experiment is on, so the logo is not an escape.
            <IconButton
                icon=gear_icon()
                aria_label="Settings"
                on_click=move |_| navigate("/settings", NavigateOptions::default())
            />
        }
            .into_any()>
            <autosync::AutosyncCard refresh=reload />
            <Card title="Packages">
                <Transition fallback=|| {
                    view! {
                        <PackageRowSkeleton />
                        <PackageRowSkeleton />
                        <PackageRowSkeleton />
                    }
                }>
                    {move || Suspend::new(async move {
                        match packages.await {
                            Ok(data) => {
                                let rows = data
                                    .packages
                                    .into_iter()
                                    .map(|p| {
                                        (
                                            p.namespace,
                                            p.state,
                                            p.provisional,
                                            p.changed_at,
                                            p.role_switch_host,
                                        )
                                    })
                                    .collect();
                                view! { <PackageList packages=rows /> }.into_any()
                            }
                            Err(err) => {
                                web_sys::console::error_1(
                                    &format!("get_main_page_packages failed: {err}").into(),
                                );
                                render_fetch_error().into_any()
                            }
                        }
                    })}
                </Transition>
            </Card>
        </PageLayout>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::StateTone;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    #[wasm_bindgen_test]
    fn renders_a_packages_card() {
        // Inside a `Router`, because `MainPage` is a routed page and its appbar asks for
        // `use_navigate`. Mounting it bare passed only while nothing in it needed router
        // context — a false premise that happened to hold.
        let el = mount(|| {
            view! {
                <leptos_router::components::Router>
                    <MainPage />
                </leptos_router::components::Router>
            }
        });
        let text = el.text_content().unwrap();
        assert!(
            text.contains("Packages"),
            "expected a Packages card, got: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn a_row_shows_the_list_wording_for_its_state() {
        let el = mount(|| {
            view! {
                <PackageList packages=vec![
                    (
                        "user/plate-07".to_string(),
                        PackageState::Behind,
                        true,
                        None,
                        None,
                    ),
                ] />
            }
        });
        let text = el.text_content().unwrap();
        assert!(text.contains("Not the latest"), "got: {text}");
        assert!(
            !text.contains("Newer revision available"),
            "that is the queue's wording; a list row must not use it"
        );
    }

    #[wasm_bindgen_test]
    fn a_row_links_to_its_own_package() {
        let el = mount(|| {
            view! {
                <PackageList packages=vec![
                    ("user/plate-07".to_string(), PackageState::Latest, true, None, None),
                ] />
            }
        });
        let href = el
            .query_selector("a[href*=installed-package]")
            .unwrap()
            .expect("the row should link to the package page")
            .get_attribute("href")
            .unwrap();
        // A bare path is the bug this pins: the package page reads the namespace from the
        // query string and reports "Invalid namespace" when it is absent.
        assert!(
            href.contains("namespace=user/plate-07"),
            "href must carry the namespace, got: {href}"
        );
    }

    #[wasm_bindgen_test]
    fn a_provisional_row_is_marked_provisional() {
        let el = mount(|| {
            view! {
                <PackageList packages=vec![
                    ("user/a".to_string(), PackageState::Latest, true, None, None),
                ] />
            }
        });
        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_some(),
            "the light phase's guess is drawn dashed until the heavy phase confirms it"
        );
    }

    #[wasm_bindgen_test]
    fn a_fetch_failure_shows_fixed_words_never_the_raw_error() {
        let el = mount(render_fetch_error);
        let text = el.text_content().unwrap();
        assert!(
            text.contains("Could not load your packages."),
            "got: {text}"
        );
        assert!(
            !text.contains("connection reset by peer"),
            "the raw backend error must not reach the page; got: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn apply_replaces_the_guess_and_clears_provisional() {
        let row = RowSignals::new(PackageState::Latest, None);
        row.apply(MainPagePackageRefreshData {
            state: PackageState::PendingChanges { files: 3 },
            role_switch_host: None,
        });

        assert_eq!(
            row.state.get_untracked(),
            PackageState::PendingChanges { files: 3 }
        );
        assert!(
            !row.provisional.get_untracked(),
            "the heavy phase confirmed it"
        );
    }

    #[wasm_bindgen_test]
    fn apply_clears_a_pre_filter_mark_the_refresh_did_not_confirm() {
        // The readable-bucket list only knows buckets registered with the stack,
        // while `set_remote` accepts any S3 bucket. Only ever ADDING the mark made
        // such a false positive permanent for the life of the page — and since the
        // mark suppresses the sign-in route, it left a genuinely broken row with no
        // remedy at all. The refresh is the real call; it gets the last word.
        let row = RowSignals::new(
            PackageState::RoleDenied {
                role: Some("ReadOnly".to_string()),
            },
            Some("test.quilt.dev".to_string()),
        );
        row.apply(MainPagePackageRefreshData {
            state: PackageState::Latest,
            role_switch_host: None,
        });

        assert_eq!(row.state.get_untracked(), PackageState::Latest);
        assert_eq!(row.role_switch_host.get_untracked(), None);
    }

    #[wasm_bindgen_test]
    fn apply_marks_a_row_the_pre_filter_cleared() {
        // And the other direction: the pre-filter says nothing about writes and
        // over-reports for unmanaged roles, so it can miss a denial the real call finds.
        let row = RowSignals::new(PackageState::Latest, None);
        row.apply(MainPagePackageRefreshData {
            state: PackageState::RoleDenied {
                role: Some("ReadOnly".to_string()),
            },
            role_switch_host: Some("test.quilt.dev".to_string()),
        });

        assert_eq!(
            row.state.get_untracked(),
            PackageState::RoleDenied {
                role: Some("ReadOnly".to_string())
            }
        );
        assert_eq!(
            row.role_switch_host.get_untracked().as_deref(),
            Some("test.quilt.dev")
        );
    }

    #[wasm_bindgen_test]
    fn a_settled_row_is_not_drawn_provisional() {
        let el = mount(|| {
            view! {
                <PackageRow
                    namespace="user/a"
                    href="/x"
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                    provisional=false
                />
            }
        });
        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_none(),
            "a confirmed state is drawn solid"
        );
    }

    #[wasm_bindgen_test]
    async fn a_row_updates_its_existing_anchor_when_state_changes() {
        // Proves the reactive props actually update in place: the same anchor
        // node carries the new words. It does not, on its own, prove that a
        // *naive* re-render would have produced a different node — Step 7's
        // bisection found tachys reconciles by structural type at a fixed
        // position, so several naive re-render patterns left `is_same_node`
        // true too. The real reason `state`/`tone` are `Signal`s rather than
        // plain values is upstream: with plain values, settling a row would
        // require re-running `PackageList`, which would re-construct
        // `PackageListRow` and re-fire its `spawn_local` refresh — a loop.
        // This test still matters as the direct check that the props this
        // task added actually behave reactively.
        let state = RwSignal::new("Latest".to_string());
        let el = mount(move || {
            view! {
                <PackageRow
                    namespace="user/a"
                    href="/x"
                    state=state
                    tone=Signal::stored(StateTone::Success)
                    provisional=true
                />
            }
        });
        let before = el.query_selector("a").unwrap().expect("row anchor");

        state.set("3 files changed".to_string());
        leptos::task::tick().await;

        let after = el.query_selector("a").unwrap().expect("row anchor");
        assert!(
            before.is_same_node(Some(&after)),
            "the row was re-created, not updated"
        );
        assert!(
            el.text_content().unwrap().contains("3 files changed"),
            "got: {}",
            el.text_content().unwrap()
        );
    }

    #[wasm_bindgen_test]
    async fn a_row_stops_being_drawn_provisional_once_it_settles() {
        // The headline behaviour: a dashed row turns solid as the heavy phase
        // confirms it, on the row it already is — this is what
        // `a_settled_row_is_not_drawn_provisional` and
        // `a_row_updates_its_existing_anchor_when_state_changes` each check
        // half of (a fixed `provisional`, and a changing `state`) but neither
        // drives the actual `true` -> `false` transition a user watches happen.
        let provisional = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <PackageRow
                    namespace="user/a"
                    href="/x"
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                    provisional=provisional
                />
            }
        });
        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_some(),
            "starts dashed, as the light phase's guess"
        );

        provisional.set(false);
        leptos::task::tick().await;

        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_none(),
            "settles to solid in place once confirmed"
        );
    }
}
