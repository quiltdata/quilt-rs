//! The v2 main page. Behind `ExperimentalSettings.main_page_v2`.
//!
//! Three regions: the state strip — Autosync beside Accounts — the attention
//! queue, and the list. The list region is two views of one roster, Packages and
//! Recent files, behind the toggle in its toolbar. `Transition`, never
//! `Suspense` (§6): a later plan
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
mod create_package;
mod grouping;
mod queue;
mod recent_files;

use std::collections::HashMap;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::commands::MainPageAccountsData;
use crate::commands::MainPagePackageData;
use crate::commands::MainPagePackageRefreshData;
use crate::commands::MainPagePackagesData;
use crate::commands::MainPageRecentFilesData;
use grouping::ListRowData;
use grouping::PackageGroup;

stylance::import_crate_style!(style, "src/pages/main_page.module.scss");

/// Copied from the gallery's own helpers rather than shared: the gallery modules are
/// not compiled into the app binary, and the kit deliberately owns no icons — a caller
/// passes the glyph, so the appbar's owner draws it.
/// A cog: a hub, a ring, and teeth that touch the ring.
///
/// The geometry is the whole icon. A small centre with long rays standing off it
/// is a sun, not a gear — so the teeth start at the ring's edge and are shorter
/// than it is wide.
fn gear_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4" stroke-linecap="round">
            <circle cx="8" cy="8" r="1.7" />
            <circle cx="8" cy="8" r="4.3" />
            <path d="M8 4.3V2.2M8 11.7v2.1M4.3 8H2.2M11.7 8h2.1" />
            <path d="M5.38 5.38 3.9 3.9M10.62 10.62l1.48 1.48M10.62 5.38 12.1 3.9M5.38 10.62 3.9 12.1" />
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
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::GroupHeading;
use crate::kit::IconButton;
use crate::kit::ListToolbar;
use crate::kit::Naming;
use crate::kit::PackageRow;
use crate::kit::PackageRowSkeleton;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::kit::Site;
use crate::kit::render;

/// The fixed sentence shown when the fetch fails. The backend's error text is
/// logged (see `render_fetch_error`) but never shown: §5's words-come-from-`render`
/// constraint applies to this file too, and a raw `Result<_, String>` error is not
/// a word from the vocabulary.
const FETCH_ERROR_WORDS: &str = "Could not load your packages.";

/// The same, for the recent-files read. A separate sentence and not the one above
/// because the two arms fail over different reads: `Could not load your packages.`
/// under the Recent files toggle is a wrong statement about which read failed, and
/// a page that misreports that is worse than one that says less. The blankslate is
/// not the alternative either — `No files yet` over a read that never answered
/// manufactures a state the page does not know.
const FILES_FETCH_ERROR_WORDS: &str = "Could not load your files.";

/// The list region's two views, as the toggle names them. Consts because each
/// string is the toggle's option, the value the selection signal holds, and the
/// condition the region switches on — three uses that must not drift apart.
const PACKAGES_VIEW: &str = "Packages";
const FILES_VIEW: &str = "Recent files";

/// The packages view's `Group` axis and the feed's, which are different
/// controls behind different signals (see `list_toolbar`'s own doc): a shared
/// signal holding `Bucket` would render the feed's select blank the moment the
/// reader switched to it, since `Bucket` is not one of its options.
const GROUP_BUCKET: &str = "Bucket";
const GROUP_PREFIX: &str = "Prefix";
const GROUP_NONE: &str = "None";
const GROUP_PACKAGE: &str = "Package";
const SORT_CHANGED: &str = "Changed";
const SORT_NAME: &str = "Name";

/// The failure branch, split out from `MainPage` so it can be tested without a
/// Tauri host. Renders only the fixed sentence for the user — logging the
/// backend's error for a developer happens once, where the fetch result is
/// handled (`MainPage`'s `Err` arm below), not here: this is a view function,
/// and view functions can re-run on every re-render, which would re-log the
/// same failure each time.
fn render_fetch_error() -> impl IntoView {
    view! { <p>{FETCH_ERROR_WORDS}</p> }
}

/// The feed's failure branch. Same shape and same reasoning as
/// [`render_fetch_error`]: only the fixed sentence reaches the user, and the
/// backend's error text is logged once where the fetch result is handled — not
/// here, because a view function can re-run on every re-render and would re-log
/// the same failure each time.
fn render_files_fetch_error() -> impl IntoView {
    view! { <p>{FILES_FETCH_ERROR_WORDS}</p> }
}

/// A row's live state: the light phase's guess, replaced in place by the heavy
/// phase's answer.
#[derive(Clone, Copy)]
struct RowSignals {
    state: RwSignal<PackageState>,
    confidence: RwSignal<Confidence>,
    role_switch_host: RwSignal<Option<String>>,
}

/// How much the page knows about a row's state.
///
/// One field rather than a `provisional` boolean beside an `unchecked` one:
/// "settled but unchecked" is not a thing, and a type that cannot express it
/// beats a note asking people not to — `kit/banner.rs` makes the same argument
/// for its own enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confidence {
    /// The heavy phase answered, and this is its answer.
    Settled,
    /// Its call is still out. Not news: the row is dashed and dimmed, and the
    /// queue waits rather than speaking (R3).
    Pending,
    /// Its call failed, so the row keeps the light phase's state — which is not
    /// wrong, only unwitnessed, and right most of the time. This is what tells
    /// "we could not look" apart from "we have not looked yet", which one
    /// boolean could not (qhq-8mgw.51).
    Unchecked,
}

impl RowSignals {
    /// `provisional` comes from the payload rather than being assumed: almost
    /// every light-phase state is a cached guess, but a `PullConflict` is the
    /// watcher's own reading of this disk and arrives already settled. Assuming
    /// it here is what dropped conflicts out of the queue offline (qhq-8mgw.40).
    fn new(state: PackageState, role_switch_host: Option<String>, provisional: bool) -> Self {
        Self {
            state: RwSignal::new(state),
            confidence: RwSignal::new(if provisional {
                Confidence::Pending
            } else {
                Confidence::Settled
            }),
            role_switch_host: RwSignal::new(role_switch_host),
        }
    }

    /// Whether anything has confirmed this state. Both unconfirmed reasons dim
    /// the row, because the dim says *unconfirmed* and that is true of both.
    fn provisional(self) -> bool {
        self.confidence.get() != Confidence::Settled
    }

    /// The heavy phase's call failed. Recorded rather than only logged, so the
    /// queue can name the packages the page could not account for.
    fn mark_unchecked(self) {
        self.confidence.set(Confidence::Unchecked);
    }

    /// A call is out for this row again. Clears a previous failure, so a retry in
    /// flight stops the cause naming a package it is currently re-checking — the
    /// queue then waits on `in_flight`, as it does for a first load (R3).
    fn mark_pending(self) {
        self.confidence.set(Confidence::Pending);
    }

    /// Replace the light phase's guess with the heavy phase's answer — in BOTH
    /// directions. The pre-filter that seeds these is an optimistic hint, and
    /// only ever adding a mark made a false positive permanent for the life of
    /// the page. The refresh is the real call, so it gets the last word.
    fn apply(self, refreshed: MainPagePackageRefreshData) {
        self.state.set(refreshed.state);
        self.role_switch_host.set(refreshed.role_switch_host);
        self.confidence.set(Confidence::Settled);
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
                    RowSignals::new(p.state.clone(), p.role_switch_host.clone(), p.provisional),
                )
            })
            .collect();
        Self {
            rows: StoredValue::new(rows),
            // One call per package. The resolve fires them, so the count doesn't
            // depend on which `Show` branch is mounted (R0).
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

    /// More calls are going out — the counterpart to [`answered`](Self::answered),
    /// for a retry, which fires calls the seed never counted.
    fn asking(&self, n: usize) {
        self.outstanding.update(|o| *o += n);
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
                if row.provisional() {
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

    /// The light-phase rows whose heavy-phase call **failed** — never the ones
    /// still waiting. `settled` drops both alike, because neither is confirmed;
    /// only these two are something the page can speak about, and the light
    /// payload is what carries the host the queue groups them by.
    fn unchecked(&self, light: &[MainPagePackageData]) -> Vec<MainPagePackageData> {
        light
            .iter()
            .filter(|p| {
                self.row(&p.namespace)
                    .is_some_and(|row| row.confidence.get() == Confidence::Unchecked)
            })
            .cloned()
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

/// What one heavy-phase call does when it returns: the state write, and the
/// answer the counter is waiting for.
///
/// The counter is decremented on **every** path, success or failure: `outstanding`
/// counts calls, and a call that failed has answered (R3).
///
/// A row whose owner has since been disposed — by a refetch, or by the view
/// toggle — absorbs the write silently, because `reactive_graph` routes a signal
/// write through `try_update` and drops the `None`. That is why nothing here
/// guards against a stale answer.
fn record_refresh(
    row: RowSignals,
    store: PackageStore,
    result: Result<MainPagePackageRefreshData, String>,
) {
    match result {
        Ok(refreshed) => row.apply(refreshed),
        Err(err) => {
            // The row keeps the light phase's state, which is honest: nothing
            // confirmed it, and it is not wrong — only unwitnessed. What the
            // failure adds is that we could not look, as distinct from not having
            // looked yet, which is what the queue names as a shared cause
            // (qhq-8mgw.51). Still logged, because the backend's own words are
            // diagnostic and never reach the page.
            row.mark_unchecked();
            web_sys::console::error_1(&format!("refresh_main_page_package failed: {err}").into());
        }
    }
    store.answered();
}

/// Re-check exactly these packages, and nothing else.
///
/// What the appbar's Refresh cannot do: that notifies the page's trigger and
/// reloads every payload. This re-fires only the heavy-phase calls a cause names,
/// which is the affordance v1 had per row and this design moved to the cause
/// (`gallery/unchecked.rs`).
///
/// The rows are looked up BEFORE the counter is raised, so a namespace with no row
/// cannot leave `outstanding` permanently above zero and the queue silent forever
/// (R3) — the same hazard the resolve's own `store.answered()` arm guards.
fn recheck(store: PackageStore, namespaces: &[String]) {
    let rows: Vec<(String, RowSignals)> = namespaces
        .iter()
        .filter_map(|namespace| Some((namespace.clone(), store.row(namespace)?)))
        .collect();
    store.asking(rows.len());
    for (namespace, row) in rows {
        row.mark_pending();
        leptos::task::spawn_local(async move {
            let result = commands::refresh_main_page_package(namespace).await;
            record_refresh(row, store, result);
        });
    }
}

/// One row: a pure view over the signals the page holds for it. The heavy-phase
/// call that fills those signals is fired by the resolve that seeded the store,
/// not here — R0 is unaffected, because it is still one call per package and a row
/// still settles where it stands by reading its own [`RowSignals`], but the number
/// of calls is now a property of the payload rather than of which subtree happens
/// to be mounted.
#[component]
fn PackageListRow(
    namespace: String,
    /// This package's live state, owned by the page's [`PackageStore`].
    row: RowSignals,
    changed_at: Option<f64>,
) -> impl IntoView {
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
            provisional=Signal::derive(move || row.provisional())
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
fn PackageList(packages: Vec<ListRowData>, store: PackageStore) -> impl IntoView {
    packages
        .into_iter()
        .filter_map(|row_data| {
            // A namespace the store was not seeded with cannot happen from one
            // payload — the rows and the store are built from the same packages —
            // and drawing nothing is not worth a panic. The counter is unaffected
            // either way: it counts the calls the resolve fired, not the rows.
            let row = store.row(&row_data.namespace)?;
            Some(view! {
                <PackageListRow
                    namespace=row_data.namespace
                    row=row
                    changed_at=row_data.changed_at
                />
            })
        })
        .collect_view()
}

/// The one cause every row in this group shares, or `None`.
///
/// Only the bucket axis has one (§3.1): a prefix spans buckets, so no cause is
/// a property of a prefix group. Reads the settled state rather than the light
/// phase's guess, for the same reason the queue does — the access pre-filter
/// over-reports, and an unconfirmed denial is not a fact about the bucket.
///
/// The words are `render`'s, at the site that states a shared cause once
/// (`Site::QueueRow`, `kit/package_state.rs:143`). `GroupHeading` draws the
/// dash.
fn group_annotation(group: &PackageGroup, store: PackageStore, group_by: &str) -> Option<String> {
    if group_by != GROUP_BUCKET || group.rows.is_empty() {
        return None;
    }
    let mut cause: Option<String> = None;
    for row in &group.rows {
        let signals = store.row(&row.namespace)?;
        if signals.provisional() {
            return None;
        }
        let state = signals.state.get();
        if !matches!(state, PackageState::RoleDenied { .. }) {
            return None;
        }
        let words = render(&state, Site::QueueRow).words;
        match &cause {
            None => cause = Some(words),
            Some(existing) if *existing == words => {}
            // Two denials naming different roles are not one shared cause.
            Some(_) => return None,
        }
    }
    cause
}

/// The list region's chrome: the toolbar, and the toggle that names which of the
/// region's two views is on screen.
///
/// A free function rather than a component so that the *same* helper can be
/// called from both arms of the queue/list boundary — its fallback and its
/// resolved view. Chrome is never skeletonised (§Loading), and the queue and the
/// list must keep sharing one boundary (R6), so rendering the toolbar twice from
/// one definition is what buys "on screen at first paint" without splitting
/// anything.
fn list_toolbar(
    view_selected: RwSignal<String>,
    query: RwSignal<String>,
    group_packages_by: RwSignal<String>,
    group_files_by: RwSignal<String>,
    sort_by: RwSignal<String>,
    create_open: RwSignal<bool>,
) -> AnyView {
    let on_packages = move || view_selected.get() == PACKAGES_VIEW;
    view! {
        <ListToolbar>
            <SegmentedControl
                aria_label="List view"
                name="main-page-view"
                options=vec![PACKAGES_VIEW.to_string(), FILES_VIEW.to_string()]
                selected=view_selected
            />
            // Two selects, not one with reactive options: `Select`'s `options` is
            // built once, and a shared `selected` holding a value the other axis
            // does not offer renders the control blank. The `SearchInput` is split
            // the same way, one instance per view guard, both bound to the same
            // `query` signal: its `aria_label` is a plain `String` (kit, read-only),
            // so it cannot say "packages" on one view and "files" on the other
            // without two instances.
            {move || {
                on_packages()
                    .then(|| {
                        view! {
                            <SearchInput
                                value=query
                                aria_label="Search packages"
                                placeholder="Search…"
                            />
                            <Select
                                naming=Naming::Prefix("Group".to_string())
                                options=vec![
                                    GROUP_BUCKET.to_string(),
                                    GROUP_PREFIX.to_string(),
                                    GROUP_NONE.to_string(),
                                ]
                                selected=group_packages_by
                            />
                            <Select
                                naming=Naming::Prefix("Sort".to_string())
                                options=vec![SORT_CHANGED.to_string(), SORT_NAME.to_string()]
                                selected=sort_by
                            />
                            <Button on_click=move |_| create_open.set(true)>
                                "Create package"
                            </Button>
                        }
                    })
            }}
            {move || {
                (!on_packages())
                    .then(|| {
                        view! {
                            <SearchInput value=query aria_label="Search files" placeholder="Search…" />
                            <Select
                                naming=Naming::Prefix("Group".to_string())
                                options=vec![GROUP_NONE.to_string(), GROUP_PACKAGE.to_string()]
                                selected=group_files_by
                            />
                        }
                    })
            }}
        </ListToolbar>
    }
    .into_any()
}

/// The Recent files view, and the inner boundary its own read needs.
///
/// A `Suspend` registers with the nearest suspense context, so without this
/// `Transition` an unresolved feed puts the queue/list boundary back into pending
/// — which re-renders its fallback and leaves two radio groups sharing one `name`
/// on the page, silently clearing the reader's selection. An inner boundary for a
/// third payload is not a split of the boundary the queue and the list share (R6).
///
/// Called from both arms of the packages read: the feed is an independent payload
/// (§5's decision 3), so a failed packages read must not take it off the page.
fn files_view(
    recent_files: LocalResource<Result<MainPageRecentFilesData, String>>,
    query: RwSignal<String>,
    group_files_by: RwSignal<String>,
) -> AnyView {
    view! {
        <Transition fallback=|| ()>
            {move || Suspend::new(async move {
                match recent_files.await {
                    Ok(data) => {
                        view! {
                            <recent_files::RecentFilesRegion
                                files=data.files
                                query=query.into()
                                group_by=group_files_by.into()
                            />
                        }
                            .into_any()
                    }
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("get_main_page_recent_files failed: {err}").into(),
                        );
                        view! { <Card>{render_files_fetch_error()}</Card> }.into_any()
                    }
                }
            })}
        </Transition>
    }
    .into_any()
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
/// # Three boundaries, and where the lines fall
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
///
/// The third is the recent-files feed's own, nested inside the list region's
/// `Show`. A `Suspend` registers with the nearest suspense context, so without a
/// boundary of its own the feed's unresolved read would put the queue/list
/// `Transition` back into pending — which re-renders its fallback and so puts a
/// second view toggle on the page, sharing the first one's radio `name` and
/// silently clearing the reader's selection. An inner boundary for a third
/// payload is not a split of the boundary the queue and the list must share.
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
    /// Stands in for `commands::get_main_page_recent_files`. The app passes
    /// nothing and the feed's own resource calls the command; a test passes a
    /// closure, because there is no Tauri host to answer that call and because
    /// counting the calls is the only way to pin R3's "fetched on the first
    /// switch, and kept thereafter".
    ///
    /// A seam for the *answer* only. Which read happens and when — the gate this
    /// component owns — stays on this side of it, so a test drives the same gate
    /// the app runs.
    #[prop(optional_no_strip)]
    fetch_files: Option<Callback<(), Result<MainPageRecentFilesData, String>>>,
) -> impl IntoView {
    // In the component body, NOT inside the `Suspend` closure below. A refetch
    // rebuilds that subtree deliberately — it is what re-collapses the queue's
    // expanders (R6) — so a view signal created inside it would reset on every
    // Refresh and throw a reader of the feed back to Packages.
    let view_selected = RwSignal::new(PACKAGES_VIEW.to_string());
    // R2, and the same reason `view_selected` is here: a refetch rebuilds the
    // resolved subtree, so a signal created inside it would clear the reader's
    // search and reset their axes on every Refresh.
    let query = RwSignal::new(String::new());
    let group_packages_by = RwSignal::new(GROUP_BUCKET.to_string());
    let group_files_by = RwSignal::new(GROUP_NONE.to_string());
    let sort_by = RwSignal::new(SORT_CHANGED.to_string());
    // Same placement, same reason: `CreatePackageDialog` mounts outside the
    // `Transition` below, and a signal created inside a refetch's resolved
    // subtree would close a dialog the reader is mid-typing into.
    let create_open = RwSignal::new(false);
    // A one-way latch, not `view_selected` itself. The resource's source closure
    // is reactive, so gating it on the view directly would refetch on *every*
    // toggle and resolve the feed back to nothing on the way to Packages —
    // discarding what R3 says must be kept. A `Memo` latches without an effect:
    // it recomputes when the view changes, but only notifies its subscriber when
    // its own value does, which happens exactly once.
    let files_wanted = Memo::new(move |prev: Option<&bool>| {
        prev.copied().unwrap_or(false) || view_selected.get() == FILES_VIEW
    });
    // §5's decision 3: separate commands exist so the page pays only for the view
    // it shows. A `LocalResource` runs its future eagerly on construction, so it
    // is the guard *inside* the future, not the resource's laziness, that keeps
    // the command from being invoked on a page that never leaves Packages.
    // `reload` is tracked too, or Refresh would leave a stale feed on screen.
    let recent_files = LocalResource::new(move || {
        reload.track();
        let wanted = files_wanted.get();
        async move {
            if !wanted {
                return Ok(MainPageRecentFilesData { files: Vec::new() });
            }
            match fetch_files {
                Some(fetch) => fetch.run(()),
                None => commands::get_main_page_recent_files().await,
            }
        }
    });

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
        // Outside every `Transition`, beside the strip: a refetch rebuilds the
        // resolved subtree below, and a dialog rebuilt mid-typing would close
        // itself. The `<dialog>` is in the browser's top layer regardless of
        // where it sits in the tree, so its position here is not a layout claim.
        <create_package::CreatePackageDialog open=create_open reload=reload />
        // The queue and the list, from one read of the package rows. This
        // `Suspend` must stay a `Suspend` — memoising it, or keeping its subtree
        // across a resolve with a `Show` or a `StoredValue`, would reuse the
        // `QueueRegion` instance and with it the expander signals a refetch is
        // supposed to reset (R6).
        <Transition fallback=move || {
            view! {
                // The toolbar, from the same helper the resolved arm calls: it is
                // on screen with the appbar and the strip, and is never itself a
                // skeleton. The skeletons below it are the packages view's,
                // because that is the view the page opens on (R4).
                <div class=style::list_region>
                    {list_toolbar(
                        view_selected,
                        query,
                        group_packages_by,
                        group_files_by,
                        sort_by,
                        create_open,
                    )}
                    <Card>
                        <PackageRowSkeleton />
                        <PackageRowSkeleton />
                        <PackageRowSkeleton />
                    </Card>
                </div>
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
                        // One heavy-phase call per light-phase package, fired by
                        // the resolve that seeded the store. `seed` sets
                        // `outstanding` to the roster's size, so the number of
                        // calls has to be a property of the payload and not of
                        // whichever subtree happens to be mounted: a resolve taken
                        // while the reader is on Recent files builds no rows at
                        // all, and a row that fired its own call left every one of
                        // them unmade.
                        //
                        // Concurrent, not serial. The two shared resources behind
                        // the call — credential vending and the `/me` role query —
                        // are already serialised in the backend, and a serial walk
                        // would make the list as slow as its slowest package while
                        // clearing every row at once, the spinner §7 rejected.
                        for package in &light {
                            let Some(row) = store.row(&package.namespace) else {
                                // Unreachable from one payload, the store having
                                // been seeded from this very list — but a package
                                // with no row fires no call, so the count it was
                                // seeded with still has to be given back (R3).
                                store.answered();
                                continue;
                            };
                            let namespace = package.namespace.clone();
                            leptos::task::spawn_local(async move {
                                let result =
                                    commands::refresh_main_page_package(namespace).await;
                                record_refresh(row, store, result);
                            });
                        }
                        let rows: Vec<ListRowData> = light
                            .iter()
                            .map(|p| ListRowData {
                                namespace: p.namespace.clone(),
                                changed_at: p.changed_at,
                                bucket: p.bucket.clone(),
                            })
                            .collect();
                        // How many packages the page holds, confirmed or not. The
                        // zero line speaks for all of them, and `settled` drops
                        // the ones no answer confirmed (R2) — including the ones
                        // whose call failed, which stop being outstanding without
                        // ever being accounted for.
                        let total = Signal::stored(light.len());
                        // §4.3's "resolved package list", at last: the queue reads
                        // what the heavy phase confirmed, not what the light phase
                        // guessed — the light phase never looks at the working
                        // tree, which is how a package with local edits ended up
                        // under an all-clear (qhq-8mgw.35). Reactive, so a row
                        // settling re-renders the queue and nothing else: the list
                        // below reads its own per-row signals, and a settle must
                        // cost nothing list-wide.
                        let unchecked_light = light.clone();
                        let settled = Signal::derive(move || store.settled(&light));
                        let unchecked = Signal::derive(move || store.unchecked(&unchecked_light));
                        let in_flight = Signal::derive(move || store.in_flight());
                        let retry =
                            Callback::new(move |namespaces: Vec<String>| {
                                recheck(store, &namespaces);
                            });
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
                            //
                            // The queue sits above both views and is untouched by
                            // the toggle below it: a queue that vanished when you
                            // looked at your files would be the opposite of an
                            // attention queue.
                            <queue::QueueRegion
                                packages=settled
                                hosts=hosts
                                in_flight=in_flight
                                total=total
                                unchecked=unchecked
                                retry=retry
                            />
                            <div class=style::list_region>
                            {list_toolbar(
                                view_selected,
                                query,
                                group_packages_by,
                                group_files_by,
                                sort_by,
                                create_open,
                            )}
                            // Neither card carries a title: the toggle immediately
                            // above names the view, and a card titled `Packages`
                            // over a Packages / Recent files switch says it twice
                            // (`Card::title`'s own doc).
                            <Show
                                when=move || view_selected.get() == FILES_VIEW
                                fallback=move || {
                                    // Cloned in the fallback's own body, not inside
                                    // the `view!`: `Card`'s children are a `move`
                                    // closure, so a clone written in there would
                                    // take `rows` out of this one and leave it
                                    // `FnOnce`, and this fallback is rebuilt every
                                    // time the reader comes back to the view.
                                    let rows = rows.clone();
                                    view! {
                                        <Card>
                                            // A search/group/sort re-arrangement, downstream
                                            // of both the seed and the resolve's call
                                            // loop above: this closure reads `query`,
                                            // `group_packages_by` and `sort_by` and no
                                            // store signal, so a settle (which writes
                                            // only per-row signals) never re-runs it and
                                            // the list is never rebuilt for that reason
                                            // (§Loading).
                                            //
                                            // R4's order: filter, then group, then sort
                                            // within each group. A heading and its rows
                                            // are flat siblings here, never wrapped in a
                                            // `div` per group — `Card`'s own
                                            // `.body > * + *` rule
                                            // (`kit/card.module.scss`) spaces any two
                                            // direct children, and a wrapper would defeat
                                            // that.
                                            {move || {
                                                let text = query.get();
                                                let filtered = grouping::filter_packages(
                                                    rows.clone(),
                                                    &text,
                                                );
                                                if filtered.is_empty() && !text.trim().is_empty() {
                                                    view! {
                                                        <Blankslate
                                                            heading=format!(
                                                                "No packages match \u{201c}{}\u{201d}",
                                                                text.trim(),
                                                            )
                                                            description="Search covers the names of packages installed on this machine."
                                                        />
                                                    }
                                                        .into_any()
                                                } else {
                                                    let group_by = group_packages_by.get();
                                                    let mut groups = grouping::group_packages(
                                                        filtered,
                                                        &group_by,
                                                    );
                                                    for group in &mut groups {
                                                        grouping::sort_within(&mut group.rows, &sort_by.get());
                                                    }
                                                    groups
                                                        .into_iter()
                                                        .map(|group| {
                                                            // R1: never written by hand — the
                                                            // count is the length of the rows
                                                            // that follow, always, including one.
                                                            let count = group.rows.len();
                                                            // The heading is its own reactive
                                                            // node, unlike the rest of this
                                                            // closure (which reads only
                                                            // `query`/`group_packages_by`/
                                                            // `sort_by`): `group_annotation` reads
                                                            // `PackageStore`'s settled signals, and
                                                            // isolating that read here — rather
                                                            // than in the closure that builds
                                                            // `groups` — is what keeps a settle
                                                            // from rebuilding anything but this one
                                                            // heading.
                                                            let heading = group.title.clone().map(|title| {
                                                                let for_annotation = PackageGroup {
                                                                    title: Some(title.clone()),
                                                                    rows: group.rows.clone(),
                                                                };
                                                                let group_by = group_by.clone();
                                                                view! {
                                                                    {move || {
                                                                        match group_annotation(
                                                                            &for_annotation,
                                                                            store,
                                                                            &group_by,
                                                                        ) {
                                                                            Some(note) => {
                                                                                view! {
                                                                                    <GroupHeading
                                                                                        title=title.clone()
                                                                                        count=count
                                                                                        annotation=note
                                                                                    />
                                                                                }
                                                                                    .into_any()
                                                                            }
                                                                            None => {
                                                                                view! {
                                                                                    <GroupHeading
                                                                                        title=title.clone()
                                                                                        count=count
                                                                                    />
                                                                                }
                                                                                    .into_any()
                                                                            }
                                                                        }
                                                                    }}
                                                                }
                                                            });
                                                            view! {
                                                                {heading}
                                                                <PackageList packages=group.rows store=store />
                                                            }
                                                        })
                                                        .collect_view()
                                                        .into_any()
                                                }
                                            }}
                                        </Card>
                                    }
                                }
                            >
                                {files_view(recent_files, query, group_files_by)}
                            </Show>
                            </div>
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
                        //
                        // The chrome and both views stand, though. The toolbar is
                        // chrome (R2), and the feed is a payload of its own that
                        // this failure says nothing about (§5's decision 3), so a
                        // reader on Recent files keeps the view they were reading
                        // and only the Packages arm carries the sentence. No title,
                        // as in the arm above.
                        view! {
                            <div class=style::list_region>
                                {list_toolbar(
                                    view_selected,
                                    query,
                                    group_packages_by,
                                    group_files_by,
                                    sort_by,
                                    create_open,
                                )}
                                <Show
                                    when=move || view_selected.get() == FILES_VIEW
                                    fallback=|| view! { <Card>{render_fetch_error()}</Card> }
                                >
                                    {files_view(recent_files, query, group_files_by)}
                                </Show>
                            </div>
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
            <PackageStatusListener reload=reload />
            <MainPageRegions packages=packages accounts=accounts reload=reload />
        </PageLayout>
    }
}

/// How long news is allowed to gather before the page asks the backend again.
///
/// A watcher tick reports every package, so news about several arrives as several
/// events a few milliseconds apart. Without a window, each would restart the read
/// the last one began.
const STATUS_BURST: std::time::Duration = std::time::Duration::from_millis(250);

/// Refetch decisions for the watcher's package-status events.
///
/// **Decisions only, never rendering** — so nothing drawn can go stale from what
/// is remembered here. v1 makes the same split for the same reason
/// (`installed_packages_list.rs:170-175`).
#[derive(Clone, Copy)]
struct StatusWatch {
    /// The last fingerprint acted on, per namespace.
    seen: StoredValue<HashMap<String, String>>,
    timer: StoredValue<Option<TimeoutHandle>>,
    reload: Trigger,
}

impl StatusWatch {
    fn new(reload: Trigger) -> Self {
        let watch = Self {
            seen: StoredValue::new(HashMap::new()),
            timer: StoredValue::new(None),
            reload,
        };
        // The pending window must not outlive the page —
        // `components/set_remote_popup.rs`'s shape for the same hazard.
        on_cleanup(move || {
            if let Some(Some(handle)) = watch.timer.try_get_value() {
                handle.clear();
            }
        });
        watch
    }

    /// Act on one event, if it reports anything the last one for its namespace
    /// did not.
    ///
    /// A namespace never seen counts as news. There is nothing to seed from — the
    /// package payload carries no fingerprint — and a swallowed first sighting
    /// would be exactly the pull that completed while the page was open.
    fn observe(self, event: &commands::PackageStatusEvent) {
        let known = self
            .seen
            .with_value(|seen| seen.get(&event.namespace) == Some(&event.fingerprint));
        if known {
            return;
        }
        self.seen.update_value(|seen| {
            seen.insert(event.namespace.clone(), event.fingerprint.clone());
        });
        if let Some(handle) = self.timer.get_value() {
            handle.clear();
        }
        let reload = self.reload;
        if let Ok(handle) = set_timeout_with_handle(move || reload.notify(), STATUS_BURST) {
            self.timer.set_value(Some(handle));
        }
    }
}

/// Ask the backend again when the watcher reports a package's upstream state has
/// changed.
///
/// Renders nothing; it exists for the subscription, which is dropped with it. Its
/// own component for [`AutosyncListener`](autosync)'s reason: registered once,
/// rather than rebuilt with a payload.
///
/// The page read this drives is the one the list rows AND the attention queue are
/// derived from, so a row cannot be patched in place the way v1 patches its own
/// (`installed_packages_list.rs:427-442`) — the queue would keep the old cause.
#[component]
fn PackageStatusListener(reload: Trigger) -> impl IntoView {
    let watch = StatusWatch::new(reload);
    let listener = crate::tauri::listen::<commands::PackageStatusEvent>(
        commands::PACKAGE_STATUS_EVENT,
        move |event| watch.observe(&event),
    );
    on_cleanup(move || drop(listener));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AccountHostData;
    use crate::commands::MainPageFileData;
    use crate::commands::MainPagePackageData;
    use crate::kit::StateTone;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
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

    /// A package the watcher has paused on a conflict. `provisional: false`
    /// because the backend sends it that way — the paused map is its own
    /// evidence, not a guess about a remote.
    fn conflicted(namespace: &str) -> MainPagePackageData {
        MainPagePackageData {
            provisional: false,
            ..pkg(
                namespace,
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string()],
                },
            )
        }
    }

    /// The list's own view of a light-phase payload — what `MainPageRegions`
    /// builds beside the store it seeds from the same packages.
    fn rows_of(packages: &[MainPagePackageData]) -> Vec<ListRowData> {
        packages
            .iter()
            .map(|p| ListRowData {
                namespace: p.namespace.clone(),
                changed_at: p.changed_at,
                bucket: p.bucket.clone(),
            })
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
            one.confidence.get_untracked() == Confidence::Pending,
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
    fn a_conflict_is_settled_on_arrival_and_reaches_the_queue_unconfirmed() {
        // qhq-8mgw.40. `settled` drops provisional rows so the access
        // pre-filter's guesses stay out of the queue, which is right — but a
        // `PullConflict` is not a guess. Assuming every light-phase row was
        // provisional meant that offline, when no heavy-phase call can answer,
        // `settled` came back empty and an unresolved conflict left the queue at
        // exactly the moment syncing could not fix it.
        //
        // This is the offline shape: nothing is settled by hand, so nothing has
        // been confirmed.
        let light = vec![
            conflicted("a/paused"),
            pkg("a/cached", PackageState::Behind),
        ];
        let store = PackageStore::seed(&light);

        let settled = store.settled(&light);

        assert_eq!(settled.len(), 1, "the conflict, and only the conflict");
        assert_eq!(settled[0].namespace, "a/paused");
        assert_eq!(
            settled[0].state,
            PackageState::PullConflict {
                files: vec!["a.csv".to_string()]
            },
            "the watcher's own reading, carried through"
        );
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
        // The list must keep settling exactly as it does today, with the signals
        // now owned a level up. `PackageList` is NOT re-rendered by a settle: a
        // row settles where it stands, through its own signals, and a settle costs
        // nothing list-wide.
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
        // R3 on the failure path. There is no Tauri host here, so every call the
        // resolve fires rejects and drives `record_refresh`'s `Err` arm — which is
        // what makes this a real test of that arm's decrement rather than of the
        // `Ok` one's. A counter only the success path decrements would leave the
        // queue waiting for an answer that is never coming.
        //
        // Driven through the page, because the page is where the calls are fired:
        // the store is seeded and its calls sent by one resolve. That the store
        // starts out waiting for all of them is
        // `the_store_holds_one_row_per_package_and_starts_them_all_provisional`.
        let payload = two_packages_all_latest();
        let (slot, on_store) = store_slot();
        let _el = mount_regions_reloading(
            Ok(payload),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;

        assert!(
            !seeded_store(slot).in_flight(),
            "R3: a failed refresh must end the waiting"
        );
    }

    #[wasm_bindgen_test]
    async fn the_list_asks_the_backend_for_nothing_of_its_own() {
        // What re-entering the Packages view costs: nothing. `Show` rebuilds its
        // branch on every flip, so while each row fired its own heavy-phase call
        // every toggle back to Packages meant one network call per package —
        // credential vending, a `/me` role query and a hash walk apiece, silently.
        // The calls belong to the resolve now, so building the list is free.
        let light = vec![
            pkg("user/plate-07", PackageState::Latest),
            pkg("user/plate-08", PackageState::Latest),
        ];
        let store = PackageStore::seed(&light);

        let el = mount(move || view! { <PackageList packages=rows_of(&light) store=store /> });
        sleep_ms(50).await;

        assert_eq!(
            el.query_selector_all("a[href*=installed-package]")
                .unwrap()
                .length(),
            2,
            "the rows really were built, so the count below is not vacuous"
        );
        assert_eq!(
            store.outstanding.get_untracked(),
            2,
            "and the list asked the backend for nothing while building them"
        );
    }

    /// Two packages on one host, one of them stuck behind a signed-out session.
    /// The queue therefore has something to say — `Needs your attention` — which
    /// is the case the ordering test must be able to fail on: a queue that renders
    /// nothing would leave the assertion with nothing to find between the strip
    /// and the list.
    ///
    /// The `provisional: false` on each row is inert, unlike the accounts fixture
    /// below: `PackageStore::seed` marks every row provisional by construction and
    /// never reads this field, so a page test settles its rows with `settle_all`.
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
        mount_regions_with_feed(packages, accounts, reload, on_store, Ok(no_files()), None)
    }

    /// [`mount_regions_reloading`] with the feed's answer, and a counter for how
    /// many times the page asked for it.
    ///
    /// The seam is the *answer* only: the resource, the latch and the `if wanted`
    /// guard are all the page's own, so what the counter counts is the page's gate
    /// (R3) and not the fixture's.
    fn mount_regions_with_feed(
        packages: Result<MainPagePackagesData, String>,
        accounts: Result<MainPageAccountsData, String>,
        reload: Trigger,
        on_store: Option<Callback<PackageStore>>,
        files: Result<MainPageRecentFilesData, String>,
        feed_calls: Option<Arc<AtomicUsize>>,
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
            let fetch_files = Callback::new(move |()| {
                if let Some(calls) = feed_calls.as_ref() {
                    calls.fetch_add(1, Ordering::Relaxed);
                }
                files.clone()
            });
            view! {
                <leptos_router::components::Router>
                    <MainPageRegions
                        packages=packages
                        accounts=accounts
                        reload=reload
                        on_store=on_store
                        fetch_files=Some(fetch_files)
                    />
                </leptos_router::components::Router>
            }
        })
    }

    /// [`mount_regions_with_feed`]'s shape with the feed's answer fixed to the
    /// given files, for a test whose subject is the feed's own search. The
    /// packages fixture is a plain two-package roster: this helper's tests care
    /// about what the feed draws, not about the packages view.
    fn mount_regions_with_files(files: Vec<MainPageFileData>) -> web_sys::Element {
        mount_regions_with_feed(
            Ok(two_packages_all_latest()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            None,
            Ok(MainPageRecentFilesData { files }),
            None,
        )
    }

    /// The page on reads that never answer, so the queue/list boundary stays on
    /// its fallback for as long as the test looks at it.
    fn mount_regions_pending() -> web_sys::Element {
        mount(|| {
            let reload = Trigger::new();
            let packages = LocalResource::new(|| {
                std::future::pending::<Result<MainPagePackagesData, String>>()
            });
            let accounts = LocalResource::new(|| {
                std::future::pending::<Result<MainPageAccountsData, String>>()
            });
            view! {
                <leptos_router::components::Router>
                    <MainPageRegions packages=packages accounts=accounts reload=reload />
                </leptos_router::components::Router>
            }
        })
    }

    /// The page whose packages read answers only after `delay_ms`, so a test can
    /// act on the chrome while the queue/list boundary is still on its fallback.
    /// The toolbar is on screen there (R2), so the toggle is reachable before the
    /// first resolve lands — which is the cold-load half of the case
    /// [`a_refetch_taken_on_the_feed_still_answers_for_every_package`] covers on
    /// the Refresh path.
    fn mount_regions_slow_packages(
        packages: MainPagePackagesData,
        accounts: MainPageAccountsData,
        on_store: Callback<PackageStore>,
        delay_ms: i32,
    ) -> web_sys::Element {
        mount(move || {
            let reload = Trigger::new();
            let packages = LocalResource::new(move || {
                let packages = packages.clone();
                async move {
                    sleep_ms(delay_ms).await;
                    Ok(packages)
                }
            });
            let accounts = LocalResource::new(move || {
                let accounts = accounts.clone();
                async move { Ok(accounts) }
            });
            let fetch_files = Callback::new(move |()| Ok(no_files()));
            view! {
                <leptos_router::components::Router>
                    <MainPageRegions
                        packages=packages
                        accounts=accounts
                        reload=reload
                        on_store=Some(on_store)
                        fetch_files=Some(fetch_files)
                    />
                </leptos_router::components::Router>
            }
        })
    }

    /// Two packages the heavy phase will agree with, so the queue's only remaining
    /// line is the zero line — and the zero line is the one thing it withholds
    /// while a call is outstanding (R3). That is what makes an unfired call
    /// visible on screen rather than only in the store.
    fn two_packages_all_latest() -> MainPagePackagesData {
        MainPagePackagesData {
            packages: vec![
                pkg("user/plate-07", PackageState::Latest),
                pkg("user/plate-08", PackageState::Latest),
            ],
        }
    }

    /// Three packages whose payload order is neither alphabetical nor by
    /// `changed_at`, so a test can drive either sort and tell it apart from a
    /// no-op: the payload order below is beta, gamma, alpha, which is not the
    /// ascending-by-name order (alpha, beta, gamma) and not the
    /// newest-first order by `changed_at` (gamma, beta, alpha) either.
    fn three_packages_out_of_order() -> MainPagePackagesData {
        MainPagePackagesData {
            packages: vec![
                MainPagePackageData {
                    namespace: "user/beta".to_string(),
                    state: PackageState::Latest,
                    changed_at: Some(5_000.0),
                    bucket: None,
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/gamma".to_string(),
                    state: PackageState::Latest,
                    changed_at: Some(9_000.0),
                    bucket: None,
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/alpha".to_string(),
                    state: PackageState::Latest,
                    changed_at: Some(1_000.0),
                    bucket: None,
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
            ],
        }
    }

    /// Two packages sharing one bucket — §3.1's default axis, and the fixture
    /// the derived-count test needs: the heading's count has to equal this
    /// fixture's own length, not a number typed by hand.
    fn two_packages_in_one_bucket() -> MainPagePackagesData {
        MainPagePackagesData {
            packages: vec![
                MainPagePackageData {
                    namespace: "user/plate-07".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("team-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/plate-08".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("team-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
            ],
        }
    }

    /// Two packages in two different buckets — one namespace matches a search
    /// for "alpha", the other does not, so a search can empty the second
    /// bucket's group without touching the first.
    fn two_packages_in_two_buckets() -> MainPagePackagesData {
        MainPagePackagesData {
            packages: vec![
                MainPagePackageData {
                    namespace: "user/alpha-plate".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("first-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/beta-plate".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("second-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
            ],
        }
    }

    /// Two packages in one bucket, a third alone in a second — the fixture the
    /// derived-count test needs to prove `GroupHeading`'s count is never typed
    /// by hand: a literal `2` passes against `two_packages_in_one_bucket`
    /// alone, but only a real `rows.len()` gets both headings right at once,
    /// including the count-of-one no other test in either region covers.
    fn two_packages_in_one_bucket_and_one_in_another() -> MainPagePackagesData {
        MainPagePackagesData {
            packages: vec![
                MainPagePackageData {
                    namespace: "user/plate-07".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("team-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/plate-08".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("team-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/solo-plate".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("solo-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
            ],
        }
    }

    /// The feed's empty answer — what the page gets before anyone has installed
    /// anything, and the default for every test whose subject is not the feed.
    fn no_files() -> MainPageRecentFilesData {
        MainPageRecentFilesData { files: Vec::new() }
    }

    /// One file in the feed, in a package the packages fixture also holds.
    fn one_recent_file() -> MainPageRecentFilesData {
        MainPageRecentFilesData {
            files: vec![MainPageFileData {
                path: "readings/plate-07.csv".to_string(),
                namespace: "user/plate-07".to_string(),
                changed_at: 1_700_000_000_000.0,
            }],
        }
    }

    /// One feed row, named after `recent_files.rs`'s own `file` test helper.
    fn file_data(path: &str, namespace: &str, changed_at: f64) -> MainPageFileData {
        MainPageFileData {
            path: path.to_string(),
            namespace: namespace.to_string(),
            changed_at,
        }
    }

    /// One option of the list toolbar's view toggle.
    ///
    /// `SegmentedControl` renders a `radiogroup` of `label`s, each wrapping a radio
    /// `input` beside a `span`, with the option's own text as the input's `value` —
    /// so the value identifies an option and the input carries the selection.
    /// Not matched on a class: `stylance` emits the module's own identifiers
    /// (`root`, `option`, `input`, `text`), none of which name this control.
    fn toggle_option(el: &web_sys::Element, label: &str) -> web_sys::HtmlInputElement {
        el.query_selector(&format!("[role=radiogroup] input[value=\"{label}\"]"))
            .unwrap()
            .unwrap_or_else(|| panic!("the list toolbar's `{label}` option"))
            .dyn_into()
            .unwrap()
    }

    /// The toolbar's search field.
    fn search_field(el: &web_sys::Element) -> web_sys::HtmlInputElement {
        el.query_selector("input[type=search]")
            .unwrap()
            .expect("the search field")
            .dyn_into()
            .unwrap()
    }

    /// Types into the search field the way a user does — set the property, then
    /// fire the event the component listens for. `SearchInput` binds `on:input`
    /// (`kit/search_input.rs:39`), so `input` is the event, not `change`.
    fn type_search(el: &web_sys::Element, text: &str) {
        let field = search_field(el);
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    /// A `<select>` in the toolbar, found by its `aria-label` rather than by
    /// position. `Select` puts `aria-label` on the `<select>` element itself,
    /// taken from `Naming::Prefix(name)` (`kit/select.rs:45`), so this is the
    /// only other selector in this file that could match one — `host_row.rs`'s
    /// `Role` select is the sole other `Select` on the page, and its label does
    /// not collide with either of these.
    fn labelled_select(el: &web_sys::Element, label: &str) -> web_sys::HtmlSelectElement {
        el.query_selector(&format!("select[aria-label='{label}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("a select labelled `{label}`"))
            .dyn_into()
            .unwrap()
    }

    fn group_select(el: &web_sys::Element) -> web_sys::HtmlSelectElement {
        labelled_select(el, "Group")
    }

    fn sort_select(el: &web_sys::Element) -> web_sys::HtmlSelectElement {
        labelled_select(el, "Sort")
    }

    /// `Select` binds `on:change` (`kit/select.rs:70`), which is what `accounts.rs`'s
    /// own role-switch test drives (`accounts.rs:192`).
    fn select_option(select: &web_sys::HtmlSelectElement, value: &str) {
        select.set_value(value);
        select
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
    }

    /// A `GroupHeading`'s own text, found by its title.
    ///
    /// `stylance` emits `<class>-<hash>` with no module prefix, so nothing on
    /// the page has `group` in a class name — `title` and `count` are shared
    /// with `Card` and `Dialog`. But `GroupHeading` is the only component that
    /// renders its title in a `<span>` (`kit/group_heading.rs:8`); `Card` and
    /// `Dialog` both use `<h2>` (`card.rs:38`, `dialog.rs:73`). So
    /// `span[class*=title]` can only match a group heading's title, and its
    /// `parent_element()` is the heading root, whose full text (title,
    /// annotation, count) this returns.
    fn heading_text(el: &web_sys::Element, title: &str) -> Option<String> {
        let spans = el.query_selector_all("span[class*=title]").unwrap();
        (0..spans.length())
            .filter_map(|i| spans.item(i))
            .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
            .find(|span| span.text_content().as_deref() == Some(title))
            .and_then(|span| span.parent_element())
            .and_then(|heading| heading.text_content())
    }

    /// Whether a `<button>` reading this text exists anywhere in `el`.
    ///
    /// Narrower than `el.text_content().contains(label)`: `CreatePackageDialog`'s
    /// own `<h2>` title reads "Create package" too, and — unlike the toolbar's
    /// button — it is in the DOM regardless of which view is on screen, since
    /// `kit::Dialog` keeps one persistent element and only toggles whether it is
    /// showing (`dialog.rs`'s `show_modal`/`close`). A button is the toolbar's own
    /// affordance, so this is what "the toolbar carries/drops Create package"
    /// actually means.
    fn has_button_labelled(el: &web_sys::Element, label: &str) -> bool {
        let buttons = el.query_selector_all("button").unwrap();
        (0..buttons.length())
            .filter_map(|i| buttons.item(i))
            .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
            .any(|b| b.text_content().unwrap_or_default().contains(label))
    }

    #[wasm_bindgen_test]
    async fn the_packages_view_toolbar_carries_every_control() {
        // §2's region 4: "The toolbar carries the view toggle, search, a `Group:`
        // select, and a `Sort:` select. `Create package` sits at its right end."
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

        assert!(
            el.query_selector("input[type=search]").unwrap().is_some(),
            "the search field"
        );
        let text = el.text_content().unwrap();
        for control in ["Group:", "Sort:"] {
            assert!(text.contains(control), "missing {control}: {text}");
        }
        // Not a `text.contains` check: `CreatePackageDialog`'s own `<h2>` title
        // also reads "Create package" and is in the DOM either way (see
        // `has_button_labelled`'s doc), so that substring is always present and
        // would not catch the toolbar's own button going missing.
        assert!(
            has_button_labelled(&el, "Create package"),
            "missing the toolbar's Create package button: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn the_feed_toolbar_drops_sort_and_create_and_keeps_its_own_group() {
        // `kit/list_toolbar.rs`'s own doc: "the packages view adds Sort and Create
        // package, the files view drops both, and Group's options differ between
        // them." Search and the view toggle are on both.
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

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("Group:"),
            "the feed groups too (§3.2): {text}"
        );
        assert!(
            !text.contains("Sort:"),
            "sort is the packages view's: {text}"
        );
        assert!(
            !has_button_labelled(&el, "Create package"),
            "create is the packages view's: {text}"
        );
        assert!(
            el.query_selector("input[type=search]").unwrap().is_some(),
            "search is on both views"
        );
    }

    #[wasm_bindgen_test]
    async fn the_feed_group_select_is_not_blank_after_a_view_switch() {
        // A native `<select>` whose value is not among its options renders blank.
        // One shared signal holding "Bucket" would do exactly that here, and the
        // control would look broken rather than throw.
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

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        assert_eq!(
            group_select(&el).value(),
            GROUP_NONE,
            "the feed's own axis defaults to None (§3.2), not to the packages axis's Bucket"
        );
    }

    #[wasm_bindgen_test]
    async fn a_refetch_leaves_the_toolbar_exactly_as_the_reader_left_it() {
        // R2. The signals live in `MainPageRegions`' body, outside the boundary a
        // refetch rebuilds — the same placement rule `view_selected` follows. Inside
        // it, pressing Refresh would clear the search box and reset the axes while
        // the reader was reading the result.
        let reload = Trigger::new();
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            reload,
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &a_package_needing_attention());
        leptos::task::tick().await;

        type_search(&el, "plate");
        select_option(&group_select(&el), GROUP_PREFIX);
        select_option(&sort_select(&el), SORT_NAME);
        sleep_ms(20).await;

        reload.notify();
        sleep_ms(50).await;

        assert_eq!(search_field(&el).value(), "plate", "the query survived");
        assert_eq!(group_select(&el).value(), GROUP_PREFIX, "the axis survived");
        assert_eq!(sort_select(&el).value(), SORT_NAME, "the sort survived");
    }

    #[wasm_bindgen_test]
    async fn choosing_sort_by_name_re_orders_the_rows_on_screen() {
        let (slot, on_store) = store_slot();
        let payload = three_packages_out_of_order();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;

        select_option(&sort_select(&el), SORT_NAME);
        sleep_ms(20).await;

        let text = el.text_content().unwrap();
        let a = text.find("user/alpha").expect("alpha");
        let b = text.find("user/beta").expect("beta");
        let c = text.find("user/gamma").expect("gamma");
        assert!(a < b && b < c, "names ascending: {text}");
    }

    #[wasm_bindgen_test]
    async fn the_packages_view_opens_grouped_by_bucket_with_a_derived_count() {
        // §3.1's default axis, and R1's rule that no count is ever written by
        // hand: the heading's count is the length of the rows under it, always
        // — including one. A fixture with only a count of two would pass
        // against a literal `2`; asserting both buckets here is what makes a
        // constant fail, and it is the only coverage in either region for a
        // group of one.
        let (slot, on_store) = store_slot();
        let payload = two_packages_in_one_bucket_and_one_in_another();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;

        let team = heading_text(&el, "s3://team-bucket").expect("the team bucket heading");
        assert!(
            team.contains("s3://team-bucket"),
            "the bucket heading: {team}"
        );
        assert!(team.contains('2'), "two rows under it: {team}");

        let solo = heading_text(&el, "s3://solo-bucket").expect("the solo bucket heading");
        assert!(
            solo.contains("s3://solo-bucket"),
            "the bucket heading: {solo}"
        );
        assert!(solo.contains('1'), "one row under it: {solo}");
    }

    #[wasm_bindgen_test]
    async fn a_search_that_empties_a_group_removes_its_heading_too() {
        // R4: filter, then group. A heading over zero rows would print a count
        // the rows contradict, which is the one thing `GroupHeading` must never
        // do.
        let (slot, on_store) = store_slot();
        let payload = two_packages_in_two_buckets();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;
        assert!(
            el.text_content().unwrap().contains("s3://second-bucket"),
            "present before the search — the pair to the absence assertion below"
        );

        // Matches only the package in the first bucket.
        type_search(&el, "alpha");
        sleep_ms(20).await;

        assert!(
            !el.text_content().unwrap().contains("s3://second-bucket"),
            "an emptied group takes its heading with it"
        );
    }

    #[wasm_bindgen_test]
    async fn a_bucket_whose_packages_all_share_a_denial_says_so_once_on_its_heading() {
        // §3.1: "A shared cause annotates its bucket group, so the list explains
        // itself without repeating an action on every row."
        let (slot, on_store) = store_slot();
        let payload = two_packages_in_one_bucket();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        let store = seeded_store(slot);
        for package in &payload.packages {
            settle(
                store,
                &package.namespace,
                PackageState::RoleDenied {
                    role: Some("analyst".to_string()),
                },
            );
        }
        leptos::task::tick().await;

        let heading = heading_text(&el, "s3://team-bucket").expect("the bucket heading");
        assert!(
            heading.contains("No access as analyst"),
            "the heading carries the shared cause: {heading}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_bucket_whose_packages_do_not_share_a_cause_gets_no_annotation() {
        // One denied package among several is not a property of the group, and an
        // annotation over it would tell the reader the whole bucket is unreachable.
        let (slot, on_store) = store_slot();
        let payload = two_packages_in_one_bucket();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        let store = seeded_store(slot);
        settle(
            store,
            &payload.packages[0].namespace,
            PackageState::RoleDenied {
                role: Some("analyst".to_string()),
            },
        );
        settle(store, &payload.packages[1].namespace, PackageState::Latest);
        leptos::task::tick().await;

        let heading = heading_text(&el, "s3://team-bucket").expect("the bucket heading");
        assert!(
            !heading.contains("No access"),
            "one denial among two is not the group's cause: {heading}"
        );
    }

    #[wasm_bindgen_test]
    async fn the_prefix_axis_never_annotates() {
        // `GroupHeading`'s own doc: "Only the bucket axis has one: a prefix spans
        // buckets, so no cause is a property of the group."
        let (slot, on_store) = store_slot();
        let payload = two_packages_in_one_bucket();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        let store = seeded_store(slot);
        for package in &payload.packages {
            settle(
                store,
                &package.namespace,
                PackageState::RoleDenied {
                    role: Some("analyst".to_string()),
                },
            );
        }
        leptos::task::tick().await;

        select_option(&group_select(&el), GROUP_PREFIX);
        sleep_ms(20).await;

        // Scoped to the heading itself, not `el`'s whole text: both fixture
        // packages are denied in the same bucket, so the queue's own R2
        // grouping (`queue.rs`'s `role_denied_groups`) states the identical
        // words as its own cause row regardless of the list's own axis — a
        // page-wide substring check would pass or fail on the queue's text,
        // never on the list heading this test means to cover.
        let heading = heading_text(&el, "user").expect("the prefix heading");
        assert!(
            !heading.contains("No access as analyst"),
            "a prefix spans buckets, so it carries no shared cause: {heading}"
        );
    }

    #[wasm_bindgen_test]
    async fn two_denials_naming_different_roles_are_not_one_shared_cause() {
        // `group_annotation`'s equality check: an implementation that dropped
        // it and just took the last row's words would pass every other test
        // here. Neither role's words may appear once they disagree.
        let (slot, on_store) = store_slot();
        let payload = two_packages_in_one_bucket();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        let store = seeded_store(slot);
        settle(
            store,
            &payload.packages[0].namespace,
            PackageState::RoleDenied {
                role: Some("analyst".to_string()),
            },
        );
        settle(
            store,
            &payload.packages[1].namespace,
            PackageState::RoleDenied {
                role: Some("curator".to_string()),
            },
        );
        leptos::task::tick().await;

        let heading = heading_text(&el, "s3://team-bucket").expect("the bucket heading");
        assert!(
            !heading.contains("No access as analyst") && !heading.contains("No access as curator"),
            "two denials naming different roles are not one shared cause: {heading}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_group_with_a_still_provisional_row_gets_no_annotation() {
        // Plan 6's own reason: the light phase's access pre-filter over-reports,
        // so a denial it guessed but the heavy phase has not yet confirmed is
        // not a fact about the bucket. The second package's light-phase state
        // is already `RoleDenied` — exactly what an over-reporting pre-filter
        // would guess — and it is never settled, so `signals.provisional` stays
        // `true`. Reusing `two_packages_in_one_bucket` (both `Latest`) would
        // not exercise the guard at all: an unsettled `Latest` row already
        // fails `group_annotation`'s `RoleDenied` match on its own, so the
        // provisional check would never be reached either way.
        let (slot, on_store) = store_slot();
        let payload = MainPagePackagesData {
            packages: vec![
                MainPagePackageData {
                    namespace: "user/plate-07".to_string(),
                    state: PackageState::Latest,
                    changed_at: None,
                    bucket: Some("team-bucket".to_string()),
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                MainPagePackageData {
                    namespace: "user/plate-08".to_string(),
                    state: PackageState::RoleDenied {
                        role: Some("analyst".to_string()),
                    },
                    changed_at: None,
                    bucket: Some("team-bucket".to_string()),
                    host: None,
                    provisional: true,
                    role_switch_host: None,
                },
            ],
        };
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        let store = seeded_store(slot);
        settle(
            store,
            &payload.packages[0].namespace,
            PackageState::RoleDenied {
                role: Some("analyst".to_string()),
            },
        );
        // `payload.packages[1]` is left provisional: no `settle` call for it,
        // even though its light-phase guess already names the same denial.
        leptos::task::tick().await;

        let heading = heading_text(&el, "s3://team-bucket").expect("the bucket heading");
        assert!(
            !heading.contains("No access"),
            "a still-provisional row is a guess, not a fact about the bucket: {heading}"
        );
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

    /// The heavy phase's answer for one row, as [`record_refresh`] would apply it
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
        //
        // Of the package half, the page still owns the `host` and the `namespace`:
        // `settled` carries those through from its own `light` payload with
        // `..p.clone()`, and the host is what the join below is about. Only the
        // `state` is written here by `settle_all`, standing in for a heavy phase
        // that has no Tauri host to answer it.
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
    async fn the_queue_waits_for_the_last_answer_before_it_says_all_is_well() {
        // R3, at the page's own call site: `in_flight` is a prop like any other,
        // and a page that wired it to a constant would pass every region-level
        // test Task 3 wrote while still announcing an all-clear it has not earned.
        //
        // No fetcher seam is needed to reach that window. `outstanding` is a plain
        // signal the store holds, so a test can put a call back in flight the way
        // a slower row would — one row answering after the others is exactly the
        // case R3 exists for, and R0 guarantees rows answer at different times.
        let payload = MainPagePackagesData {
            packages: vec![
                pkg("user/plate-07", PackageState::Latest),
                pkg("user/plate-08", PackageState::Latest),
            ],
        };
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;

        let store = seeded_store(slot);
        // Both rows confirmed and both Latest, so the queue derives nothing and
        // the zero line is the only thing left for it to draw — but one answer is
        // still outstanding, so it may not draw it yet.
        settle_all(store, &payload);
        store.outstanding.set(1);
        leptos::task::tick().await;
        let text = el.text_content().unwrap();
        assert!(
            !text.contains("Everything is Latest"),
            "R3: an answer is still outstanding, so the all-clear is not a fact yet: {text}"
        );

        store.outstanding.set(0);
        leptos::task::tick().await;
        let text = el.text_content().unwrap();
        assert!(
            text.contains("Everything is Latest — 2 packages"),
            "and it says so the moment the last answer lands: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_package_the_page_could_not_read_holds_back_the_all_clear() {
        // The other door into a false all-clear, and the one `in_flight` alone
        // cannot close. `outstanding` decrements on failure by design (R3), and a
        // failed row stays provisional so R2 drops it from `settled` — so a page
        // that could not read three of its forty-three packages arrives at
        // "nothing is outstanding, nothing needs attention" and would announce
        // "Everything is Latest — 40 packages" over three rows still drawn dashed
        // in the list below. A signed-out host is the ordinary way to get there:
        // `refresh_main_page_package` propagates a login error rather than
        // degrading it, precisely so the row stays dashed.
        //
        // The zero line speaks for every package, so it waits for every package.
        let payload = MainPagePackagesData {
            packages: vec![
                pkg("user/plate-07", PackageState::Latest),
                pkg("user/unreachable", PackageState::Latest),
            ],
        };
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;

        let store = seeded_store(slot);
        // One answered, one whose call failed: still provisional, and no longer
        // outstanding. That combination is the whole of this test.
        settle(store, "user/plate-07", PackageState::Latest);
        store.outstanding.set(0);
        leptos::task::tick().await;

        let text = el.text_content().unwrap();
        assert!(
            !text.contains("Everything is Latest"),
            "one package could not be read, so the page has not earned this: {text}"
        );

        // And the moment it can be read, the sentence appears — naming both
        // packages, not the one that happened to answer first.
        settle(store, "user/unreachable", PackageState::Latest);
        leptos::task::tick().await;
        let text = el.text_content().unwrap();
        assert!(
            text.contains("Everything is Latest — 2 packages"),
            "every package accounted for, and the count matches the list: {text}"
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

    #[wasm_bindgen_test]
    async fn the_toolbar_and_its_list_share_one_parent() {
        // `PageLayout`'s column sets the gap BETWEEN regions, so the toolbar and
        // the list it names have to be one child of that column. Left as siblings
        // they each take a region's share of the gap, and the toolbar ends up
        // further from its own list than it is from the queue above it — region 4
        // drawn as two.
        //
        // This pins the NESTING and nothing more. No stylesheet is loaded in the
        // test harness, so `getComputedStyle` here returns browser defaults and
        // the spacing itself cannot be asserted: swapping the wrapper to
        // `display: contents` keeps this test green while the gap comes back.
        // Manual check 82 on `qhq-8mgw.21` is the other half.
        let (slot, on_store) = store_slot();
        let payload = a_package_needing_attention();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;

        let region = el
            .query_selector("[class*=list_region]")
            .unwrap()
            .expect("the list region wraps the pair");
        assert!(
            region
                .query_selector("input[type=search]")
                .unwrap()
                .is_some(),
            "the toolbar is inside it"
        );
        assert!(
            region
                .query_selector("a[href*=namespace]")
                .unwrap()
                .is_some(),
            "and so is the list it names"
        );
        // The region's stylesheet makes its LAST child the scroller, so the count
        // is load-bearing: a third child here would either take the overflow
        // itself or leave the card unscrollable, and neither shows up in a test
        // that only asks whether the two halves are present.
        assert_eq!(
            region.query_selector_all(":scope > *").unwrap().length(),
            2,
            "toolbar then card, and nothing else"
        );
    }

    #[wasm_bindgen_test]
    async fn the_page_opens_on_packages_and_the_toggle_switches_to_files() {
        // R4: no persistence, so every load opens on Packages — and the toggle
        // really swaps the view rather than stacking a second one under it.
        let el = mount_regions(Ok(a_package_needing_attention()), Ok(one_signed_out_host()));
        sleep_ms(50).await;
        // Scoped to the row's own link, not the page's whole text:
        // `CreatePackageDialog` is mounted permanently (Task 7) and its first
        // `FormControl` caption reads "…user/plate-07." unconditionally, so an
        // unscoped `contains` here would pass even if the page opened on the
        // feed.
        assert!(
            el.query_selector("a[href*='namespace=user/plate-07']")
                .unwrap()
                .is_some(),
            "the packages view is the default (R4)"
        );

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        let text = el.text_content().unwrap();
        assert!(text.contains("No files yet"), "got: {text}");
        assert!(
            !text.contains("Latest"),
            "the packages view's rows are gone, not merely hidden: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn the_queue_survives_a_view_switch() {
        // The toggle is region 4's. A queue that vanished when you looked at your
        // files would be the opposite of an attention queue.
        //
        // Settled first, because the queue draws only what the heavy phase
        // confirmed (R2) and there is no Tauri host here to confirm anything.
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
        let before = queue_text(&el).expect("the queue has something to say");

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        assert_eq!(
            queue_text(&el).as_deref(),
            Some(before.as_str()),
            "the queue is untouched by the list's own toggle"
        );
    }

    #[wasm_bindgen_test]
    async fn the_toolbar_is_on_screen_before_the_packages_read_resolves() {
        // Chrome is never skeletonised. The toolbar renders in the queue/list
        // boundary's fallback as well as in its resolved arm — which is what buys
        // "on screen at first paint" without splitting a boundary the queue and
        // the list must share.
        let el = mount_regions_pending();
        // One tick and no more. `mount_to` does not paint a suspense boundary's
        // fallback synchronously — the container is still empty when it returns —
        // so this is the first paint there is. The reads never answer, so whatever
        // is on screen from here on is the fallback's.
        leptos::task::tick().await;
        let text = el.text_content().unwrap();
        assert!(text.contains(FILES_VIEW), "the toggle is chrome: {text}");
        assert!(
            el.query_selector("[class*=skeleton]").unwrap().is_some(),
            "and the read really is still pending, so the claim above is not vacuous"
        );
        assert!(
            toggle_option(&el, PACKAGES_VIEW).checked(),
            "a live control and not a skeleton of one, opened on Packages (R4)"
        );
    }

    #[wasm_bindgen_test]
    async fn a_refetch_leaves_the_reader_where_they_were() {
        // The view signal lives in `MainPageRegions`' body, outside the subtree a
        // refetch rebuilds — the same placement rule the queue's expander map
        // follows, for the opposite reason. A signal created inside the resolved
        // subtree would send a reader of the feed back to Packages every time they
        // pressed Refresh.
        //
        // `prop:checked`, not an attribute: `SegmentedControl` sets the DOM
        // property, so the property is what the assertion reads.
        let reload = Trigger::new();
        let el = mount_regions_reloading(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            reload,
            None,
        );
        sleep_ms(50).await;
        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;
        assert!(toggle_option(&el, FILES_VIEW).checked(), "on the feed");
        assert_eq!(
            el.query_selector_all("[role=radiogroup]").unwrap().length(),
            1,
            "one toolbar on the page: two radio groups sharing a `name` are one \
             group, and the second would clear the reader's selection"
        );

        reload.notify();
        sleep_ms(50).await;

        assert!(
            toggle_option(&el, FILES_VIEW).checked(),
            "still on the feed after a Refresh"
        );
        assert!(
            el.text_content().unwrap().contains("No files yet"),
            "and it is the feed that is drawn, not the packages view"
        );
    }

    #[wasm_bindgen_test]
    async fn the_feed_is_read_when_it_is_first_asked_for_and_then_kept() {
        // R3, both halves. §5's decision 3 gives the feed a command of its own so
        // the page pays only for the view it shows — a `LocalResource` runs its
        // future eagerly, so what defers the read is the guard inside that future,
        // not the resource. And the read is *kept*: gating on the view itself
        // would refetch on every toggle and resolve the feed back to empty on the
        // way to Packages, which is the opposite of "a second switch is instant".
        let calls = Arc::new(AtomicUsize::new(0));
        let reload = Trigger::new();
        let el = mount_regions_with_feed(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            reload,
            None,
            Ok(one_recent_file()),
            Some(calls.clone()),
        );
        sleep_ms(50).await;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "the page opened on Packages and paid nothing for the feed"
        );

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1, "asked for, so read once");
        assert!(
            el.text_content().unwrap().contains("plate-07.csv"),
            "got: {}",
            el.text_content().unwrap()
        );

        toggle_option(&el, PACKAGES_VIEW).click();
        sleep_ms(50).await;
        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "kept, so a second switch is instant"
        );
        assert!(
            el.text_content().unwrap().contains("plate-07.csv"),
            "and the feed still holds what it read"
        );

        reload.notify();
        sleep_ms(50).await;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "Refresh reads it again, or the feed on screen would be stale"
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_files_read_says_which_read_failed() {
        // The two views fail over two different reads, so they cannot share one
        // sentence: `Could not load your packages.` under the Recent files toggle
        // is a wrong statement about what went wrong. The negative half is the
        // point — it is what catches a later refactor collapsing the two arms back
        // onto one helper.
        //
        // The packages read answers here, so the only failure on the page is the
        // feed's, and the packages sentence has no honest way to appear.
        let el = mount_regions_with_feed(
            Ok(a_package_needing_attention()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            None,
            Err("connection reset by peer".to_string()),
            None,
        );
        sleep_ms(50).await;
        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains(FILES_FETCH_ERROR_WORDS),
            "the feed says its own read failed: {text}"
        );
        assert!(
            !text.contains(FETCH_ERROR_WORDS),
            "and not the packages sentence, over a packages read that answered: {text}"
        );
        assert!(
            !text.contains("connection reset by peer"),
            "the raw backend error must not reach the page: {text}"
        );
        assert!(
            !text.contains("No files yet"),
            "a read that never answered is not an empty feed: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_refetch_taken_on_the_feed_still_answers_for_every_package() {
        // The heavy phase is fired by the resolve that seeded the store, not by
        // the rows that draw it. `Show` builds exactly one branch, so a Refresh
        // taken on Recent files builds no rows at all: while each row fired its own
        // call, that resolve made none, `outstanding` stayed at the roster's size
        // for as long as the reader stayed on the feed, and the queue — which sits
        // above both views — went silent until they switched back.
        //
        // Read through the zero line, because that is the sentence R3 makes
        // conditional on the count: with every row confirmed and nothing to
        // report, an outstanding call is the only thing left that can withhold it.
        let payload = two_packages_all_latest();
        let (slot, on_store) = store_slot();
        let reload = Trigger::new();
        let el = mount_regions_reloading(
            Ok(payload.clone()),
            Ok(one_signed_out_host()),
            reload,
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;
        assert!(
            el.text_content().unwrap().contains("Everything is Latest"),
            "the queue speaks before the switch: {}",
            el.text_content().unwrap()
        );

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;
        reload.notify();
        // Long enough for the new resolve's calls to be fired and to fail: there
        // is no Tauri host, so each one drives `record_refresh`'s `Err` arm.
        sleep_ms(50).await;
        // The heavy phase's part, which a test has to play for the new store the
        // refetch seeded exactly as it does for the first one.
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("Everything is Latest — 2 packages"),
            "and it is still on the page after a Refresh taken on the feed: {text}"
        );
        assert!(
            toggle_option(&el, FILES_VIEW).checked(),
            "with the reader still on the feed, which is where they were"
        );
    }

    #[wasm_bindgen_test]
    async fn picking_the_feed_before_the_first_resolve_still_answers_for_every_package() {
        // The same defect on the cold-load path. The toolbar is chrome and is on
        // screen while the packages read is still pending (R2), so the reader can
        // already be on Recent files by the time the resolve lands and seeds the
        // store — and a store whose calls are fired by its rows is then seeded
        // with a count nothing will ever give back.
        let payload = two_packages_all_latest();
        let (slot, on_store) = store_slot();
        let el = mount_regions_slow_packages(payload.clone(), one_signed_out_host(), on_store, 100);
        leptos::task::tick().await;
        assert!(
            el.query_selector("[class*=skeleton]").unwrap().is_some(),
            "the packages read really has not answered yet"
        );

        toggle_option(&el, FILES_VIEW).click();
        // Past the read's own delay, and past its calls answering.
        sleep_ms(250).await;
        settle_all(seeded_store(slot), &payload);
        leptos::task::tick().await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("Everything is Latest — 2 packages"),
            "the queue arrives even though the list never drew a row: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_packages_read_leaves_the_feed_reachable() {
        // The two views are two payloads (§5's decision 3), so one read failing
        // must not take the other view off the page. A reader on Recent files who
        // presses Refresh into a failed packages read would otherwise lose both
        // their feed and the toggle back to it, and be shown a sentence about
        // packages they were not looking at.
        let el = mount_regions_with_feed(
            Err("connection reset by peer".to_string()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            None,
            Ok(one_recent_file()),
            None,
        );
        sleep_ms(50).await;
        assert!(
            el.text_content().unwrap().contains(FETCH_ERROR_WORDS),
            "the packages arm carries the failure"
        );

        // Panics if the toolbar is not there, which is the other half of this.
        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("plate-07.csv"),
            "and the feed, whose own read answered, is still reachable: {text}"
        );
        assert!(
            !text.contains(FETCH_ERROR_WORDS),
            "without the packages sentence left standing over it: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn coming_back_to_packages_keeps_what_the_heavy_phase_confirmed() {
        // A view switch rebuilds the rows — `Show` reconstructs its children — but
        // it must not re-seed the store: `PackageStore::seed` runs in the `Suspend`
        // arm above the `Show`, so what the heavy phase confirmed survives. A
        // re-seed would put every row back to provisional and the queue back into
        // its waiting state on nothing more than a look at the files view.
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
        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_none(),
            "every row is confirmed before the switch"
        );
        let queue_before = queue_text(&el).expect("the queue has something to say");

        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;
        toggle_option(&el, PACKAGES_VIEW).click();
        sleep_ms(50).await;

        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_none(),
            "and still confirmed after the round trip: the store was not re-seeded"
        );
        assert_eq!(
            queue_text(&el).as_deref(),
            Some(queue_before.as_str()),
            "so the queue never went back to waiting"
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
    fn an_answer_whose_row_is_gone_is_dropped_and_still_counts() {
        // What replaced the cancellation flag the row used to carry. A refetch, or
        // a view switch, disposes the owner the previous resolve's row signals
        // belong to, and the call that resolve fired can still answer afterwards.
        // Writing a disposed signal is a silent no-op, so the answer needs nothing
        // to tell it to stay quiet — but the count still has to land, or the queue
        // waits for an answer that already came (R3).
        let store = PackageStore::seed(&[pkg("user/a", PackageState::Latest)]);
        let owner = Owner::new();
        let row = owner.with(|| RowSignals::new(PackageState::Latest, None, true));
        owner.cleanup();
        assert!(
            row.state.try_get_untracked().is_none(),
            "the row really is disposed, so what follows is not vacuous"
        );

        record_refresh(
            row,
            store,
            Ok(MainPagePackageRefreshData {
                state: PackageState::Behind,
                role_switch_host: None,
            }),
        );

        assert_eq!(
            store.outstanding.get_untracked(),
            0,
            "the call answered, whatever became of the row it was for"
        );
    }

    #[wasm_bindgen_test]
    fn an_answer_writes_its_row_and_ends_that_call_s_waiting() {
        // The other half, so the test above cannot pass on a `record_refresh` that
        // never writes anything at all.
        let store = PackageStore::seed(&[pkg("user/a", PackageState::Latest)]);
        let row = store.row("user/a").expect("seeded");

        record_refresh(
            row,
            store,
            Ok(MainPagePackageRefreshData {
                state: PackageState::Behind,
                role_switch_host: None,
            }),
        );

        assert_eq!(store.outstanding.get_untracked(), 0);
        assert_eq!(row.state.get_untracked(), PackageState::Behind);
        assert!(
            row.confidence.get_untracked() == Confidence::Settled,
            "the heavy phase confirmed it"
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
        let row = RowSignals::new(PackageState::Latest, None, true);
        row.apply(MainPagePackageRefreshData {
            state: PackageState::PendingChanges { files: 3 },
            role_switch_host: None,
        });

        assert_eq!(
            row.state.get_untracked(),
            PackageState::PendingChanges { files: 3 }
        );
        assert!(
            row.confidence.get_untracked() == Confidence::Settled,
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
            true,
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
        let row = RowSignals::new(PackageState::Latest, None, true);
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
        // plain values is upstream: with plain values, settling one row would
        // require re-running `PackageList` and rebuilding every row on the
        // page. This test still matters as the direct check that the props
        // behave reactively.
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

    #[wasm_bindgen_test]
    async fn searching_narrows_the_packages_view_and_says_so_when_nothing_matches() {
        let (slot, on_store) = store_slot();
        let el = mount_regions_reloading(
            Ok(two_packages_all_latest()),
            Ok(one_signed_out_host()),
            Trigger::new(),
            Some(on_store),
        );
        sleep_ms(50).await;
        settle_all(seeded_store(slot), &two_packages_all_latest());
        leptos::task::tick().await;

        // The positive half: a query matching one of the two rows keeps that
        // row and drops the other, rather than the assertion below being the
        // test's only real claim.
        type_search(&el, "plate-07");
        sleep_ms(20).await;
        assert!(
            el.query_selector("a[href*='namespace=user/plate-07']")
                .unwrap()
                .is_some(),
            "the matching row stays"
        );
        assert!(
            el.query_selector("a[href*='namespace=user/plate-08']")
                .unwrap()
                .is_none(),
            "the non-matching row is dropped"
        );

        type_search(&el, "plate-99");
        sleep_ms(20).await;

        let text = el.text_content().unwrap();
        assert!(
            text.contains("No packages match \u{201c}plate-99\u{201d}"),
            "the gallery's own words, with the query in them: {text}"
        );
        assert!(
            text.contains("Search covers the names of packages installed on this machine."),
            "got: {text}"
        );
    }

    #[wasm_bindgen_test]
    async fn a_search_that_empties_the_list_does_not_touch_the_queue() {
        // The toolbar belongs to region 4. Filtering changes what the LIST draws
        // and nothing else — a queue that emptied when you searched would be
        // hiding the thing the page exists to show you.
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
        let before = queue_text(&el).expect("the queue has something to say");

        type_search(&el, "matches-nothing-at-all");
        sleep_ms(20).await;

        assert_eq!(
            queue_text(&el).as_deref(),
            Some(before.as_str()),
            "the queue is untouched by the list's own search"
        );
    }

    #[wasm_bindgen_test]
    async fn searching_the_feed_matches_paths_and_says_so_when_nothing_matches() {
        // R5's other half, and the feed's own words (`gallery/recent_files.rs:263`).
        let el = mount_regions_with_files(vec![file_data("runs/a/one.csv", "user/alpha", 1_000.0)]);
        sleep_ms(50).await;
        toggle_option(&el, FILES_VIEW).click();
        sleep_ms(50).await;

        type_search(&el, "runs/a");
        sleep_ms(20).await;
        assert!(
            el.text_content().unwrap().contains("one.csv"),
            "a path match"
        );

        type_search(&el, "plate-99");
        sleep_ms(20).await;
        let text = el.text_content().unwrap();
        assert!(
            text.contains("No files match \u{201c}plate-99\u{201d}"),
            "got: {text}"
        );
        assert!(
            text.contains("Files that exist only in a bucket are not included."),
            "got: {text}"
        );
    }

    /// One status event, carrying the two fields a refetch decision reads.
    fn status_event(namespace: &str, fingerprint: &str) -> commands::PackageStatusEvent {
        commands::PackageStatusEvent {
            namespace: namespace.to_string(),
            status: "behind".to_string(),
            has_changes: false,
            fingerprint: fingerprint.to_string(),
        }
    }

    /// The page's exact wiring: a `StatusWatch` over the trigger a counting read
    /// tracks. `autosync`'s `the_pages_refresh_asks_the_backend_again` pattern, for
    /// its stated reason — a refetch's only observable is the fetch.
    ///
    /// The watch is built inside the mount because its `StoredValue`s need an
    /// owner, and handed back out because it is `Copy`.
    fn mount_status_watch() -> (StatusWatch, RwSignal<i32>) {
        let calls = RwSignal::new(0);
        let out: std::rc::Rc<std::cell::Cell<Option<StatusWatch>>> =
            std::rc::Rc::new(std::cell::Cell::new(None));
        let sink = std::rc::Rc::clone(&out);
        mount(move || {
            let reload = Trigger::new();
            let packages = LocalResource::new(move || {
                reload.track();
                calls.update(|n| *n += 1);
                async move {
                    Ok::<MainPagePackagesData, String>(MainPagePackagesData { packages: vec![] })
                }
            });
            sink.set(Some(StatusWatch::new(reload)));
            view! {
                <Transition fallback=|| ()>
                    {move || {
                        Suspend::new(async move {
                            let _ = packages.await;
                        })
                    }}
                </Transition>
            }
        });
        (out.get().expect("the watch, built during the mount"), calls)
    }

    #[wasm_bindgen_test]
    async fn a_first_sighting_asks_the_backend_again() {
        // The package payload carries no fingerprint, so there is nothing to seed
        // the watch from: a namespace it has never seen must count as news, or the
        // pull that completes while the page is open — the case this exists for —
        // is exactly the one it would swallow.
        let (watch, calls) = mount_status_watch();
        sleep_ms(50).await;
        let before = calls.get_untracked();
        assert_eq!(before, 1, "the first read, before any event");

        watch.observe(&status_event("user/pkg", "fp-1"));
        sleep_ms(400).await;

        assert_eq!(
            calls.get_untracked() - before,
            1,
            "a first sighting must ask the backend again"
        );
    }

    #[wasm_bindgen_test]
    async fn a_changed_fingerprint_asks_the_backend_again() {
        // The bug this exists for: autopull brings a package up to date while the
        // page is open, and the rows and the queue derived from them keep the
        // values read at mount until something asks again.
        let (watch, calls) = mount_status_watch();
        sleep_ms(50).await;
        watch.observe(&status_event("user/pkg", "fp-1"));
        sleep_ms(400).await;
        let before = calls.get_untracked();

        watch.observe(&status_event("user/pkg", "fp-2"));
        sleep_ms(400).await;

        assert_eq!(
            calls.get_untracked() - before,
            1,
            "a changed tree must ask the backend again"
        );
    }

    #[wasm_bindgen_test]
    async fn a_repeated_fingerprint_leaves_the_page_alone() {
        // `report_status` fires per package per TICK, not per change, and carries
        // the fingerprint precisely so a consumer can discard repeats. Acting on
        // every event would re-run the two-phase package read — heavy phase,
        // network and hashing — at tick rate.
        let (watch, calls) = mount_status_watch();
        sleep_ms(50).await;
        watch.observe(&status_event("user/pkg", "fp-1"));
        sleep_ms(400).await;
        let before = calls.get_untracked();

        for _ in 0..3 {
            watch.observe(&status_event("user/pkg", "fp-1"));
        }
        sleep_ms(400).await;

        assert_eq!(
            calls.get_untracked(),
            before,
            "an unchanged tree must not rebuild the page"
        );
    }

    #[wasm_bindgen_test]
    async fn one_ticks_burst_costs_one_refetch() {
        // A tick reports every package, so news about three of them arrives as
        // three events a few milliseconds apart. Each restarting the read would
        // leave the page fetching its own payload three times over.
        let (watch, calls) = mount_status_watch();
        sleep_ms(50).await;
        let before = calls.get_untracked();

        watch.observe(&status_event("user/a", "fp-a"));
        watch.observe(&status_event("user/b", "fp-b"));
        watch.observe(&status_event("user/c", "fp-c"));
        sleep_ms(400).await;

        assert_eq!(
            calls.get_untracked() - before,
            1,
            "one tick's news costs one refetch, not one per package"
        );
    }
    #[wasm_bindgen_test]
    fn unchecked_names_only_the_rows_whose_check_failed() {
        // qhq-8mgw.51. Three rows, three fates, and the third is why one boolean
        // was not enough: a row still waiting is NOT news — the queue waits for
        // it (R3) — while a row whose call failed is something the page can
        // speak about. A fixture without the waiting row would not tell them
        // apart.
        let light = vec![
            pkg("a/confirmed", PackageState::Latest),
            pkg("a/failed", PackageState::Latest),
            pkg("a/waiting", PackageState::Latest),
        ];
        let store = PackageStore::seed(&light);
        store
            .row("a/confirmed")
            .unwrap()
            .apply(MainPagePackageRefreshData {
                state: PackageState::Latest,
                role_switch_host: None,
            });
        record_refresh(
            store.row("a/failed").unwrap(),
            store,
            Err("no route to host".to_string()),
        );

        let unchecked = store.unchecked(&light);
        assert_eq!(unchecked.len(), 1, "only the one whose check failed");
        assert_eq!(unchecked[0].namespace, "a/failed");
    }

    #[wasm_bindgen_test]
    fn a_failed_check_keeps_the_cached_state_and_stays_out_of_settled() {
        // The failure is not a state: the package is whatever the light phase
        // said, only unwitnessed. Overwriting it would throw away the
        // informative half, which is the argument for a cause row rather than
        // words in the row's state label.
        let light = vec![pkg("a/failed", PackageState::Behind)];
        let store = PackageStore::seed(&light);
        record_refresh(
            store.row("a/failed").unwrap(),
            store,
            Err("credential vending failed".to_string()),
        );

        assert_eq!(
            store.row("a/failed").unwrap().state.get_untracked(),
            PackageState::Behind,
            "a failed check must not overwrite what the light phase knew"
        );
        assert!(
            store.settled(&light).is_empty(),
            "and nothing unconfirmed reaches the queue as a fact (R2)"
        );
    }
}
