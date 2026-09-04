//! The v2 main page. Behind `ExperimentalSettings.main_page_v2`.
//!
//! Three regions: the state strip — Autosync beside Accounts — the attention
//! queue, and the package list. `Transition`, never `Suspense` (§6): a later plan
//! will wire a refetch on every autosync transition, publish and pause, and a
//! `Suspense` boundary re-shows its fallback each time, so the page would strobe.
//! Today `reload` fires from the Refresh button and from a role switch, which
//! moves the package rows as surely as it moves the Accounts card.
//!
//! # One read of each payload, held here
//!
//! §1: resolution happens once, upstream of every region. The queue is derived
//! from the same package rows the list draws and the same host facts the Accounts
//! card draws, so both light-phase resources live on the page rather than inside
//! the region that happens to draw them first — a region fetching its own copy
//! would ask the same question twice and could be told two different answers.
//! Both are awaited in one place and handed down as plain values.

mod accounts;
mod autosync;
mod queue;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::commands::MainPageAccountsData;
use crate::commands::MainPagePackageData;
use crate::commands::MainPagePackageRefreshData;
use crate::commands::MainPagePackagesData;

stylance::import_crate_style!(style, "src/pages/main_page.module.scss");

/// What a row needs that its live state does not carry: which package it is, and
/// when the copy last changed. Everything that settles — the state, whether it is
/// still the light phase's guess, the host whose role selector its switch
/// affordance would open — now lives in [`PackageStore`], keyed by this namespace.
/// A tuple rather than a struct because it is local to this file and never crosses
/// a boundary.
type PackageRowData = (String, Option<f64>);

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

/// Every row's live state, held by the page instead of by the list row that draws
/// it. The heavy phase's answer is what the attention queue is about — a package
/// with uncommitted changes is exactly what the queue exists to name — and while
/// those signals were private to each [`PackageListRow`] no other region could
/// read them.
///
/// Two `Copy` handles and nothing else, so the store costs nothing to capture in
/// as many closures as the page has readers.
#[derive(Clone, Copy)]
struct PackageStore {
    /// One entry per light-phase package, keyed by namespace. A `StoredValue`
    /// rather than a signal: seeding fixes the keys for the life of the payload,
    /// and only the signals inside an entry ever change — so nothing that reads
    /// the map should re-run when a row settles.
    rows: StoredValue<HashMap<String, RowSignals>>,
    /// Heavy-phase calls not yet answered. R3: the queue may not claim an
    /// all-clear while any of them is outstanding, and `provisional` cannot
    /// carry that — a failed refresh stays provisional forever.
    outstanding: RwSignal<usize>,
}

impl PackageStore {
    /// One row per light-phase package, each holding that phase's guess.
    fn seed(packages: &[MainPagePackageData]) -> Self {
        let rows = packages
            .iter()
            .map(|p| {
                (
                    p.namespace.clone(),
                    RowSignals::new(p.state.clone(), p.role_switch_host.clone()),
                )
            })
            .collect();
        Self {
            rows: StoredValue::new(rows),
            // One call per package, because the list mounts one row per package
            // and each row fires its own (R0).
            outstanding: RwSignal::new(packages.len()),
        }
    }

    /// This package's live state, or `None` for a namespace the store was not
    /// seeded with.
    fn row(&self, namespace: &str) -> Option<RowSignals> {
        self.rows.with_value(|rows| rows.get(namespace).copied())
    }

    /// One heavy-phase call answered, successfully or not.
    fn answered(&self) {
        // `saturating_sub`, not `-= 1`: a double-decrement would be a bug in the
        // caller, and underflowing a `usize` in a release build wraps to a number
        // that leaves the queue silent forever (R3).
        self.outstanding.update(|n| *n = n.saturating_sub(1));
    }

    /// Whether any heavy-phase call is still outstanding (R3).
    fn in_flight(&self) -> bool {
        self.outstanding.get() > 0
    }

    /// The light-phase payload with the heavy phase's answers written over it,
    /// dropping every row the heavy phase has not confirmed (R2). The access
    /// pre-filter over-reports, so a guess it made must not reach the queue as a
    /// denial (qhq-8mgw.35).
    ///
    /// Reads the signals with `.get()`, never `get_untracked()`: the caller's
    /// reactivity is the entire point of holding these signals on the page, and
    /// "optimising" this to an untracked read would leave the queue frozen on the
    /// light phase again — `a_reader_of_settled_re_runs_when_a_row_settles` is
    /// what catches that.
    fn settled(&self, light: &[MainPagePackageData]) -> Vec<MainPagePackageData> {
        light
            .iter()
            .filter_map(|p| {
                let row = self.row(&p.namespace)?;
                if row.provisional.get() {
                    return None;
                }
                Some(MainPagePackageData {
                    state: row.state.get(),
                    role_switch_host: row.role_switch_host.get(),
                    provisional: false,
                    ..p.clone()
                })
            })
            .collect()
    }
}

/// The package's own page, `namespace` in the query string because
/// `installed_package` reads it with `use_query_map` and a bare path leaves it
/// empty. Shared by the list row's own link below and by `queue::action_href`'s
/// `[Get latest]` / `[Choose S3 bucket]` arms, which land here because neither
/// action has a page of its own (v1 puts `Pull` in its status banner and
/// `SetRemote` in its toolbar, both on this page). `installed_packages_list.rs`
/// (v1, read-only) builds the identical string by hand — this helper is v2's
/// only copy.
fn package_page_href(namespace: &str) -> String {
    format!("/installed-package?namespace={namespace}&filter=unmodified")
}

/// One row, with its own heavy-phase refresh. Firing one `refresh_main_page_package`
/// call per row, rather than a single call for the whole page, is what makes a row
/// settle where it stands instead of the page clearing all at once (Ruling R0).
#[component]
fn PackageListRow(
    namespace: String,
    /// This package's live state, owned by the page's [`PackageStore`].
    row: RowSignals,
    /// Notified when this row's heavy-phase call answers, successfully or not
    /// (R3): the queue may not claim an all-clear while a call is outstanding,
    /// and a failed call must stop the waiting too.
    answered: Callback<()>,
    changed_at: Option<f64>,
) -> impl IntoView {
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
        // Both arms, and only past the cancellation check above: the call has
        // answered either way, and a row unmounted mid-flight must neither write
        // nor decrement.
        answered.run(());
    });

    let href = package_page_href(&namespace);
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
fn PackageList(packages: Vec<PackageRowData>, store: PackageStore) -> impl IntoView {
    // The list holds the store, so the list is what can hand a row the counter —
    // the row itself is given only its own signals.
    let answered = Callback::new(move |()| store.answered());
    packages
        .into_iter()
        .filter_map(|(namespace, changed_at)| {
            // A namespace the store was not seeded with cannot happen from one
            // payload — the rows and the store are built from the same packages —
            // and drawing nothing is not worth a panic.
            let Some(row) = store.row(&namespace) else {
                // But the counter still has to balance: no row means no call and
                // so no answer, and an `outstanding` that never reaches zero
                // leaves the queue silent for good (R3).
                answered.run(());
                return None;
            };
            Some(view! {
                <PackageListRow
                    namespace=namespace
                    row=row
                    answered=answered
                    changed_at=changed_at
                />
            })
        })
        .collect_view()
}

/// The page's three regions, in the order they are read: the state strip, the
/// attention queue, then the package list. §2's arrangement, and the reason the
/// queue sits above the list — it is what you look at first.
///
/// # Why this takes resources and not payloads
///
/// The strip has to be constructed **exactly once**. Its Autosync card owns a
/// resource of its own whose fetcher tracks this same trigger, so a card rebuilt
/// by a refetch would ask the backend a second time for the payload the standing
/// card is already refetching — and worse, the new card's `Transition` has no
/// previous body to hold, so it would render its empty fallback until the second
/// read returned. The strip would blank on every Refresh, which is exactly the
/// strobe §6's Transition-never-Suspense rule exists to forbid.
///
/// So the boundaries live **inside** this component rather than around it, and
/// what it takes is the page's two resources. A test supplies its own —
/// `LocalResource::new(|| async { Ok(fixture()) })` resolves with no Tauri host —
/// which is what makes all three regions visible to an assertion at once.
///
/// # Two boundaries, and where the line falls
///
/// The strip's Accounts card gets its own small `Transition`, so the strip does
/// not wait for the package rows and each card holds its previous body through a
/// refetch. The queue and the list share one, because the queue is derived from
/// the same rows the list draws: separate boundaries would let the list arrive
/// above a queue still deciding whether it has anything to say.
///
/// A refetch rebuilds that subtree, and the rebuild is what re-collapses an
/// expanded cause group (R6): `QueueRegion` builds its expander map when it is
/// constructed — and each group's signal lazily, the first time a render derives
/// that cause — so a region kept across a refetch would leave a group open over
/// rows the new payload no longer holds. Whether the queue's `packages` input is
/// reactive is orthogonal to that: R6 asks that the region be *constructed*
/// again, not that what it reads sit still.
#[component]
fn MainPageRegions(
    /// The page's package read. Awaited once, by the boundary the queue and the
    /// list share.
    packages: LocalResource<Result<MainPagePackagesData, String>>,
    /// The page's accounts read. Awaited by both boundaries — the Accounts card
    /// draws it and the queue joins against it (§4.3, R3) — and fetched once:
    /// awaiting a resource reads its value, it does not re-run its fetcher.
    accounts: LocalResource<Result<MainPageAccountsData, String>>,
    /// The page's reload trigger, which every resource here tracks.
    reload: Trigger,
    /// Handed the [`PackageStore`] the moment the page seeds one, so a test can
    /// drive a settle the way the heavy phase would. Read-only and one-shot: the
    /// page still owns the store and still seeds it from its own payload, so a
    /// caller cannot make the test's page differ from the app's. The app passes
    /// nothing; there is no Tauri host in a test, so every row's real refresh
    /// fails and nothing would ever settle without this.
    ///
    /// `optional_no_strip` rather than `optional`: the plain form makes the
    /// builder take a bare `Callback`, and the one caller that passes anything
    /// here already holds an `Option`.
    #[prop(optional_no_strip)]
    on_store: Option<Callback<PackageStore>>,
) -> impl IntoView {
    view! {
        // Outside every boundary, so both cards are constructed once and each one
        // owns when it blanks.
        <div class=style::strip>
            <autosync::AutosyncCard refresh=reload />
            <Transition fallback=|| ()>
                {move || Suspend::new(async move {
                    match accounts.await {
                        Ok(data) => {
                            view! { <accounts::AccountsBody data=data refresh=reload /> }
                                .into_any()
                        }
                        Err(err) => {
                            // Logged here, once, and rendered as nothing: asserting
                            // anything about a user's sessions on the strength of a
                            // failed read would be a manufactured state.
                            web_sys::console::error_1(
                                &format!("get_main_page_accounts failed: {err}").into(),
                            );
                            ().into_any()
                        }
                    }
                })}
            </Transition>
        </div>
        // The queue and the list, from one read of the package rows. This
        // `Suspend` must stay a `Suspend` — memoising it, or keeping its subtree
        // across a resolve with a `Show` or a `StoredValue`, would reuse the
        // `QueueRegion` instance and with it the expander signals a refetch is
        // supposed to reset (R6).
        <Transition fallback=|| {
            view! {
                <Card title="Packages">
                    <PackageRowSkeleton />
                    <PackageRowSkeleton />
                    <PackageRowSkeleton />
                </Card>
            }
        }>
            {move || Suspend::new(async move {
                match packages.await {
                    Ok(data) => {
                        // The light phase's payload, held for the life of this
                        // resolve: it is the base the store's projection writes
                        // the heavy phase's answers over, and both the store and
                        // the list's rows are built from it. Moved rather than
                        // cloned — the page is the only owner and `rows` is taken
                        // before the projection captures it.
                        let light = data.packages;
                        // Seeded here rather than inside the list: the heavy
                        // phase's answers belong to the page, so the queue can
                        // read them too.
                        let store = PackageStore::seed(&light);
                        if let Some(on_store) = on_store {
                            on_store.run(store);
                        }
                        let rows: Vec<PackageRowData> = light
                            .iter()
                            .map(|p| (p.namespace.clone(), p.changed_at))
                            .collect();
                        // §4.3's "resolved package list", at last: the queue reads
                        // what the heavy phase confirmed, not what the light phase
                        // guessed — the light phase never looks at the working
                        // tree, which is how a package with local edits ended up
                        // under an all-clear (qhq-8mgw.35). Reactive, so a row
                        // settling re-renders the queue and nothing else: the list
                        // below keeps reading its own per-row signals, because
                        // re-running `PackageList` would re-fire every row's
                        // refresh.
                        let settled = Signal::derive(move || store.settled(&light));
                        let in_flight = Signal::derive(move || store.in_flight());
                        // The accounts read is awaited here too, for the queue's
                        // join — inside this arm, because a page with no rows has
                        // no queue to join anything to. Its failure is not logged a
                        // second time (the strip's branch above handles that) and
                        // leaves the queue with no host facts, so no cause can be
                        // attributed to a host and those packages fall to rows of
                        // their own. What is unknown is which hosts are signed out.
                        let hosts = accounts.await.map(|data| data.hosts).unwrap_or_default();
                        view! {
                            // No wrapper and no margin: `PageLayout`'s column owns
                            // the gap between regions, and the queue is a direct
                            // child of it like the other two.
                            <queue::QueueRegion
                                packages=settled
                                hosts=hosts
                                in_flight=in_flight
                            />
                            <Card title="Packages">
                                <PackageList packages=rows store=store />
                            </Card>
                        }
                            .into_any()
                    }
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("get_main_page_packages failed: {err}").into(),
                        );
                        // No queue at all, rather than a zero line: `Everything is
                        // Latest — 0 packages` over a read that never answered is a
                        // manufactured all-clear.
                        view! {
                            <Card title="Packages">{render_fetch_error()}</Card>
                        }
                            .into_any()
                    }
                }
            })}
        </Transition>
    }
}

#[component]
pub fn MainPage() -> impl IntoView {
    let reload = Trigger::new();
    let packages = LocalResource::new(move || {
        reload.track();
        async move { commands::get_main_page_packages().await }
    });
    // Held here rather than inside the Accounts card, which is where it used to
    // live: the queue joins against these same host facts (§4.3, R3), and a second
    // resource for them would be a second read of one question.
    let accounts = LocalResource::new(move || {
        reload.track();
        commands::get_main_page_accounts()
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
            <MainPageRegions packages=packages accounts=accounts reload=reload />
        </PageLayout>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AccountHostData;
    use crate::commands::MainPagePackageData;
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

    /// One light-phase row, in any state the caller names. `provisional: true`
    /// as the light phase always delivers it, and every other field empty: the
    /// store's tests are about the signals, not about the payload's trimmings.
    fn pkg(namespace: &str, state: PackageState) -> MainPagePackageData {
        MainPagePackageData {
            namespace: namespace.to_string(),
            state,
            changed_at: None,
            bucket: None,
            host: None,
            provisional: true,
            role_switch_host: None,
        }
    }

    /// The list's own view of a light-phase payload — what `MainPageRegions`
    /// builds beside the store it seeds from the same packages.
    fn rows_of(packages: &[MainPagePackageData]) -> Vec<PackageRowData> {
        packages
            .iter()
            .map(|p| (p.namespace.clone(), p.changed_at))
            .collect()
    }

    #[wasm_bindgen_test]
    fn the_store_holds_one_row_per_package_and_starts_them_all_provisional() {
        let store = PackageStore::seed(&[
            pkg("a/one", PackageState::Latest),
            pkg("a/two", PackageState::Behind),
        ]);

        let one = store.row("a/one").expect("seeded from this payload");
        assert_eq!(one.state.get_untracked(), PackageState::Latest);
        assert!(
            one.provisional.get_untracked(),
            "the light phase's guess is provisional by construction"
        );
        assert!(store.row("b/absent").is_none());
        assert_eq!(store.outstanding.get_untracked(), 2, "one call per package");
    }

    #[wasm_bindgen_test]
    fn a_settled_row_leaves_the_store_still_in_flight_until_the_last_one_answers() {
        // The zero line may not appear while any answer is outstanding (R3), and
        // `provisional` cannot carry that: a failed refresh stays provisional forever.
        let store = PackageStore::seed(&[
            pkg("a/one", PackageState::Latest),
            pkg("a/two", PackageState::Latest),
        ]);
        assert!(store.in_flight());

        store.answered();
        assert!(store.in_flight(), "one call is still outstanding");
        store.answered();
        assert!(!store.in_flight());
    }

    #[wasm_bindgen_test]
    fn settled_drops_what_the_heavy_phase_has_not_confirmed() {
        // R2, and the half of qhq-8mgw.35 that manufactures a denial: the access
        // pre-filter over-reports, so its guesses must not reach the queue.
        let light = vec![
            pkg("a/confirmed", PackageState::Latest),
            pkg("a/guessed", PackageState::RoleDenied { role: None }),
        ];
        let store = PackageStore::seed(&light);
        store
            .row("a/confirmed")
            .unwrap()
            .apply(MainPagePackageRefreshData {
                state: PackageState::PendingChanges { files: 1 },
                role_switch_host: None,
            });

        let settled = store.settled(&light);
        assert_eq!(settled.len(), 1, "only the confirmed one");
        assert_eq!(settled[0].namespace, "a/confirmed");
        assert_eq!(
            settled[0].state,
            PackageState::PendingChanges { files: 1 },
            "the heavy phase's answer, not the light phase's guess"
        );
    }

    #[wasm_bindgen_test]
    async fn a_reader_of_settled_re_runs_when_a_row_settles() {
        // `settled` reads its signals with `.get()` and must keep doing so. Its
        // only other input is a `StoredValue`, which is not reactive, so an
        // "optimising" `get_untracked()` would leave a `Signal::derive` over it
        // with no dependencies at all — and the queue, which is now that
        // derivation's only reader, would freeze on the light phase. That is
        // qhq-8mgw.35 exactly, so this is the whole plan's regression guard.
        let light = vec![pkg("user/plate-07", PackageState::Latest)];
        let store = PackageStore::seed(&light);
        let settled = Signal::derive(move || store.settled(&light));
        let el = mount(move || view! { <p>{move || settled.get().len()}</p> });
        assert_eq!(
            el.text_content().unwrap(),
            "0",
            "nothing is confirmed yet (R2)"
        );

        settle(store, "user/plate-07", PackageState::Behind);
        leptos::task::tick().await;

        assert_eq!(
            el.text_content().unwrap(),
            "1",
            "the settle reached the reader; `settled` still reads through .get()"
        );
    }

    #[wasm_bindgen_test]
    async fn a_row_reads_the_store_and_settles_in_place() {
        // The list must keep settling exactly as it does today, with the signals now
        // owned a level up. `PackageList` is NOT re-rendered by a settle — that would
        // re-fire every row's refresh (see the module's own comment).
        let light = vec![pkg("user/plate-07", PackageState::Latest)];
        let store = PackageStore::seed(&light);
        let el = mount(move || view! { <PackageList packages=rows_of(&light) store=store /> });
        assert!(el.text_content().unwrap().contains("Latest"));

        store
            .row("user/plate-07")
            .unwrap()
            .apply(MainPagePackageRefreshData {
                state: PackageState::Behind,
                role_switch_host: None,
            });
        leptos::task::tick().await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("Not the latest"),
            "the list's wording: {text}"
        );
        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_none(),
            "settled rows are drawn solid"
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_refresh_ends_the_waiting_too() {
        // R3 on the failure path. There is no Tauri host here, so the `catch`-bound
        // invoke rejects and every mounted row drives the `Err` arm — which is what
        // makes this a real test of that arm's decrement rather than of the `Ok`
        // one's. A counter only the success path decrements would leave the queue
        // waiting for an answer that is never coming.
        let light = vec![pkg("user/plate-07", PackageState::Latest)];
        let store = PackageStore::seed(&light);
        assert!(store.in_flight(), "the row's call has not answered yet");

        let _el = mount(move || view! { <PackageList packages=rows_of(&light) store=store /> });
        sleep_ms(50).await;

        assert!(
            !store.in_flight(),
            "R3: a failed refresh must end the waiting"
        );
    }

    /// Two packages on one host, one of them stuck behind a signed-out session.
    /// The queue therefore has something to say — `Needs your attention` — which
    /// is the case the ordering test must be able to fail on: a queue that renders
    /// nothing would leave the assertion with nothing to find between the strip
    /// and the list.
    fn a_package_needing_attention() -> MainPagePackagesData {
        MainPagePackagesData {
            packages: vec![
                MainPagePackageData {
                    namespace: "user/plate-07".to_string(),
                    state: PackageState::Unknown,
                    changed_at: None,
                    bucket: None,
                    host: Some("solo.registry.io".to_string()),
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/plate-08".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: None,
                    host: Some("solo.registry.io".to_string()),
                    provisional: false,
                    role_switch_host: None,
                },
            ],
        }
    }

    /// The host the fixture above points at, signed out — the other half of R3's
    /// join. Settled (`provisional: false`), because a provisional row spawns an
    /// invoke that can only fail without a Tauri host.
    fn one_signed_out_host() -> MainPageAccountsData {
        MainPageAccountsData {
            hosts: vec![AccountHostData {
                host: "solo.registry.io".to_string(),
                signed_in: false,
                current_role: None,
                roles: Vec::new(),
                provisional: false,
            }],
        }
    }

    /// The page's body on two resolving reads, inside a `Router` because both
    /// `AccountRow` and `QueueRegion` ask for `use_navigate`.
    ///
    /// `MainPageRegions` takes the page's resources rather than their payloads —
    /// the strip has to be constructed exactly once, so the boundaries live inside
    /// it — so a test hands it resources of its own. A `LocalResource` over a ready
    /// future resolves with no Tauri host, which is what puts all three regions on
    /// screen at once.
    fn mount_regions(
        packages: Result<MainPagePackagesData, String>,
        accounts: Result<MainPageAccountsData, String>,
    ) -> web_sys::Element {
        mount_regions_reloading(packages, accounts, Trigger::new(), None)
    }

    /// [`mount_regions`] with the caller's own trigger, for the test that drives a
    /// refetch, and with the store seam, for the tests that drive a settle. Both
    /// resources track the trigger, exactly as the page's own do.
    fn mount_regions_reloading(
        packages: Result<MainPagePackagesData, String>,
        accounts: Result<MainPageAccountsData, String>,
        reload: Trigger,
        on_store: Option<Callback<PackageStore>>,
    ) -> web_sys::Element {
        mount(move || {
            let packages = LocalResource::new(move || {
                reload.track();
                let packages = packages.clone();
                async move { packages }
            });
            let accounts = LocalResource::new(move || {
                reload.track();
                let accounts = accounts.clone();
                async move { accounts }
            });
            view! {
                <leptos_router::components::Router>
                    <MainPageRegions
                        packages=packages
                        accounts=accounts
                        reload=reload
                        on_store=on_store
                    />
                </leptos_router::components::Router>
            }
        })
    }

    /// A slot for the store the page seeds, and the callback that fills it. An
    /// `RwSignal` rather than an `Rc<Cell<_>>` because `Callback::new` wants a
    /// `Send + Sync` closure.
    fn store_slot() -> (RwSignal<Option<PackageStore>>, Callback<PackageStore>) {
        let slot = RwSignal::new(None);
        (slot, Callback::new(move |store| slot.set(Some(store))))
    }

    /// The store the page most recently seeded. A refetch seeds a new one, so a
    /// test that reloads must read this again afterwards.
    fn seeded_store(slot: RwSignal<Option<PackageStore>>) -> PackageStore {
        slot.get_untracked().expect("the page seeded a store")
    }

    /// The heavy phase's answer for one row, as [`PackageListRow`] would apply it
    /// if there were a Tauri host to answer the call.
    fn settle(store: PackageStore, namespace: &str, state: PackageState) {
        store
            .row(namespace)
            .expect("the store was seeded with this namespace")
            .apply(MainPagePackageRefreshData {
                state,
                role_switch_host: None,
            });
    }

    /// Every row confirmed with exactly what the light phase guessed — the heavy
    /// phase agreeing. The queue draws only confirmed rows (R2) and no row can
    /// confirm itself without a Tauri host, so a page test whose subject is not
    /// the settle still has to play that part or assert against an empty queue.
    fn settle_all(store: PackageStore, packages: &MainPagePackagesData) {
        for package in &packages.packages {
            store
                .row(&package.namespace)
                .expect("seeded from this payload")
                .apply(MainPagePackageRefreshData {
                    state: package.state.clone(),
                    role_switch_host: package.role_switch_host.clone(),
                });
        }
    }

    /// The queue card's own text, or `None` when the region drew nothing.
    ///
    /// Scoped to the card rather than taken from the whole page because the list
    /// row below says some of the same words — `render(&state, Site::ListRow)`
    /// gives `1 file changed` too — so an unscoped `contains` would pass on a
    /// queue that never heard about the settle. Found by its title rather than by
    /// a class: `stylance` emits `Card`'s own identifiers, which are `root`,
    /// `title` and `body` for every card on the page.
    fn queue_text(el: &web_sys::Element) -> Option<String> {
        let sections = el.query_selector_all("section").unwrap();
        (0..sections.length())
            .filter_map(|i| sections.item(i))
            .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
            .filter_map(|section| section.text_content())
            .find(|text| text.contains("Needs your attention"))
    }

    /// The strip element itself, so a test can ask what is inside it rather than
    /// only where its text falls in the document.
    fn strip_of(el: &web_sys::Element) -> web_sys::Element {
        el.query_selector("[class*=strip]")
            .unwrap()
            .expect("the state strip")
    }

    #[wasm_bindgen_test]
    async fn the_queue_sits_between_the_strip_and_the_list() {
        // Section 2's arrangement, and the reason the queue exists above the list:
        // it is what you look at first.
        //
        // The word asserted for the strip is `Accounts` rather than `Autosync`:
        // `Autosync` lives inside `AutosyncBody`, behind that card's own resource,
        // which has no Tauri host to answer it here.
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &a_package_needing_attention());
        leptos::task::tick().await;

        let html = el.inner_html();
        let strip = html.find("Accounts").expect("strip");
        let queue = html
            .find("Everything is Latest")
            .or_else(|| html.find("Needs your attention"))
            .expect("queue");
        let list = html.find("Packages").expect("list");
        assert!(
            strip < queue && queue < list,
            "strip, then queue, then list"
        );

        // Index order alone cannot tell "between the two regions" from "a third
        // card inside the strip" — the first occurrence of `Accounts` still
        // precedes a queue rendered after `AccountsBody` and inside the same
        // `div`. So ask the strip what it holds.
        let strip = strip_of(&el).text_content().unwrap();
        assert!(
            strip.contains("Accounts"),
            "the strip is what was found: {strip}"
        );
        assert!(
            !strip.contains("Needs your attention"),
            "the queue is a region of the page, not a card in the strip: {strip}"
        );
    }

    #[wasm_bindgen_test]
    async fn the_queue_is_drawn_from_the_same_payloads_as_the_cards() {
        // §1, at the seam: one package read feeds the queue and the list, and one
        // accounts read feeds the Accounts card and the queue's join. The queue's
        // cause names the host the accounts payload says is signed out — which
        // needs both halves of R3's join — and the list still holds every row.
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &a_package_needing_attention());
        leptos::task::tick().await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("Signed out from solo.registry.io"),
            "the queue joined the packages against the accounts payload: {text}"
        );
        assert!(
            strip_of(&el)
                .text_content()
                .unwrap()
                .contains("solo.registry.io"),
            "and the same accounts payload drew the card"
        );
        assert_eq!(
            el.query_selector_all("a[href*=installed-package]")
                .unwrap()
                .length(),
            2,
            "and the list still draws every row of the same package payload"
        );
    }

    #[wasm_bindgen_test]
    async fn the_queue_names_a_package_the_heavy_phase_found_changes_in() {
        // qhq-8mgw.35, end to end and at the page level: the operator saw
        // "Everything is Latest" above a package with uncommitted changes. The
        // light phase cannot see the working tree, so only the heavy phase's
        // answer can name this package — and until this task the queue was never
        // told about it.
        //
        // The accounts fixture is the signed-out host, which the packages here
        // cannot join to: `pkg` leaves `host` at `None`, so no cause is ever
        // attributed and every row that reaches the queue is a row of its own.
        // That is the point — this test is about the state, not the join.
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(MainPagePackagesData {
                packages: vec![
                    pkg("user/plate-07", PackageState::Latest),
                    pkg("user/other", PackageState::Latest),
                ],
            }),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        // Every row is still provisional — the refreshes this mount fired have
        // no Tauri host to answer them — so `settled` is empty and the region
        // takes its `packages.is_empty()` early return. Silence for that reason,
        // not for R3's in-flight guard, which `queue.rs`'s own tests pin: by now
        // every one of those failed calls has decremented the counter.
        assert!(!el.text_content().unwrap().contains("Everything is Latest"));

        settle(
            seeded_store(slot),
            "user/plate-07",
            PackageState::PendingChanges { files: 1 },
        );
        settle(seeded_store(slot), "user/other", PackageState::Latest);
        leptos::task::tick().await;

        let queue = queue_text(&el).expect("the queue has something to say");
        assert!(
            queue.contains("1 file changed"),
            "the queue names it: {queue}"
        );
        assert!(queue.contains("Publish"), "beside its action: {queue}");
        let text = el.text_content().unwrap();
        assert!(
            !text.contains("Everything is Latest"),
            "and no longer claims otherwise: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_packages_read_leaves_no_queue_to_claim_all_is_well() {
        // A failed read is not an empty one: `Everything is Latest — 0 packages`
        // over a fetch that never answered is a manufactured all-clear. The strip
        // is unaffected, because it is outside that boundary.
        let el = mount_regions(
            Err("connection reset by peer".to_string()),
            Ok(one_signed_out_host()),
        );
        sleep_ms(50).await;

        let text = el.text_content().unwrap();
        assert!(text.contains(FETCH_ERROR_WORDS), "got: {text}");
        assert!(
            !text.contains("connection reset by peer"),
            "the raw backend error must not reach the page: {text}"
        );
        assert!(
            !text.contains("Everything is Latest"),
            "nothing answered; the page cannot say everything is fine: {text}"
        );
        assert!(
            strip_of(&el).text_content().unwrap().contains("Accounts"),
            "and the strip still stands: it is outside that boundary"
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_accounts_read_still_draws_the_queue_and_the_list() {
        // The other direction. Without host facts no cause can be attributed to a
        // host, so the signed-out package falls to a row of its own rather than
        // vanishing — and the Accounts card renders nothing at all rather than a
        // card with no rows, which would assert the user has no sessions.
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(a_package_needing_attention()),
            Err("nope".to_string()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        // Settled, so the queue really does draw — otherwise "no host was said to
        // be signed out" would hold over a region that drew nothing at all.
        settle_all(seeded_store(slot), &a_package_needing_attention());
        leptos::task::tick().await;
        assert!(
            queue_text(&el).is_some(),
            "the queue drew, so the assertion below is about what it says"
        );

        let text = el.text_content().unwrap();
        assert!(
            !text.contains("Signed out from"),
            "nothing said any host was signed out: {text}"
        );
        assert!(
            !strip_of(&el).text_content().unwrap().contains("Accounts"),
            "no card, rather than an empty one: {text}"
        );
        assert_eq!(
            el.query_selector_all("a[href*=installed-package]")
                .unwrap()
                .length(),
            2,
            "the list is drawn from its own read, which answered"
        );
    }

    #[wasm_bindgen_test]
    async fn a_refetch_rebuilds_the_queue_rather_than_reusing_it() {
        // R6, driven through the page's own machinery: the trigger the Refresh
        // button notifies, the resources that track it, and the `Suspend` that
        // re-runs when they resolve. An expanded cause group re-collapses because
        // `QueueRegion` is constructed again and builds new expander signals — a
        // memoised subtree, or one held across the resolve, would keep the old
        // ones and this would stay open.
        let (slot, on_store) = store_slot();
        let reload = Trigger::new();
        let el = mount_regions_reloading(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            reload,
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &a_package_needing_attention());
        leptos::task::tick().await;

        let expander = el
            .query_selector("[aria-expanded]")
            .unwrap()
            .expect("the cause row's expander");
        expander.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
        leptos::task::tick().await;
        assert_eq!(
            el.query_selector("[aria-expanded]")
                .unwrap()
                .unwrap()
                .get_attribute("aria-expanded")
                .as_deref(),
            Some("true"),
            "the group is open before the refetch"
        );

        reload.notify();
        sleep_ms(50).await;
        // The refetch re-seeds: the new store's rows are provisional again, so
        // the heavy phase has to answer again before the queue has anything to
        // draw. Settling here is also what keeps this test honest — a group that
        // re-collapsed only because its row vanished would prove nothing.
        settle_all(seeded_store(slot), &a_package_needing_attention());
        leptos::task::tick().await;

        assert_eq!(
            el.query_selector("[aria-expanded]")
                .unwrap()
                .expect("the cause row survives the refetch")
                .get_attribute("aria-expanded")
                .as_deref(),
            Some("false"),
            "a refetch rebuilds the region, which re-collapses the group"
        );
    }

    #[wasm_bindgen_test]
    async fn the_accounts_resource_is_fetched_once_per_load_and_once_per_reload() {
        // Finding I4: the plan's one structural judgement — one accounts
        // `LocalResource` awaited in both the strip's `Transition` and the
        // queue's `Suspend` — is untested. `mount_regions_reloading` already
        // constructs the resources inside the test, so a fetcher that
        // increments a shared counter pins the invocation count directly: 1
        // on mount, however many places read the resolved value, and one
        // more per `reload.notify()`, never one per boundary that awaits it.
        let calls = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let reload = Trigger::new();
        let fetch_calls = calls.clone();
        let _el = mount(move || {
            let packages = LocalResource::new(move || {
                reload.track();
                async move { Ok::<_, String>(a_package_needing_attention()) }
            });
            let accounts_data = one_signed_out_host();
            let accounts = LocalResource::new(move || {
                reload.track();
                let fetch_calls = fetch_calls.clone();
                let accounts_data = accounts_data.clone();
                async move {
                    fetch_calls.set(fetch_calls.get() + 1);
                    Ok::<_, String>(accounts_data)
                }
            });
            view! {
                <leptos_router::components::Router>
                    <MainPageRegions packages=packages accounts=accounts reload=reload />
                </leptos_router::components::Router>
            }
        });
        sleep_ms(50).await;
        assert_eq!(
            calls.get(),
            1,
            "one fetch on mount, however many boundaries await it"
        );

        reload.notify();
        sleep_ms(50).await;
        assert_eq!(
            calls.get(),
            2,
            "one more fetch per reload, not one per Transition/Suspend that reads it"
        );
    }

    /// A promise-backed sleep, the same four lines over `set_timeout` that
    /// [`accounts`](super::accounts)'s tests use.
    async fn sleep_ms(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    #[wasm_bindgen_test]
    async fn renders_a_packages_card() {
        // Inside a `Router`, because `MainPage` is a routed page and its appbar asks for
        // `use_navigate`. Mounting it bare passed only while nothing in it needed router
        // context — a false premise that happened to hold.
        //
        // Awaited, because there is no Tauri host here: both of the page's reads
        // reject, and the list region is drawn by the seam on the far side of that.
        // So this is the whole failure path end to end — the page still draws the
        // region, carrying the fixed sentence rather than the backend's error.
        let el = mount(|| {
            view! {
                <leptos_router::components::Router>
                    <MainPage />
                </leptos_router::components::Router>
            }
        });
        sleep_ms(200).await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("Packages"),
            "expected a Packages card, got: {text}"
        );
        assert!(
            text.contains(FETCH_ERROR_WORDS),
            "and the fixed sentence for a read that failed, got: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn a_row_shows_the_list_wording_for_its_state() {
        let light = vec![pkg("user/plate-07", PackageState::Behind)];
        let store = PackageStore::seed(&light);
        let el = mount(move || view! { <PackageList packages=rows_of(&light) store=store /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("Not the latest"), "got: {text}");
        assert!(
            !text.contains("Newer revision available"),
            "that is the queue's wording; a list row must not use it"
        );
    }

    #[wasm_bindgen_test]
    fn a_row_links_to_its_own_package() {
        let light = vec![pkg("user/plate-07", PackageState::Latest)];
        let store = PackageStore::seed(&light);
        let el = mount(move || view! { <PackageList packages=rows_of(&light) store=store /> });
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
        let light = vec![pkg("user/a", PackageState::Latest)];
        let store = PackageStore::seed(&light);
        let el = mount(move || view! { <PackageList packages=rows_of(&light) store=store /> });
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
