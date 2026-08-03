mod content;
mod entries;
mod selection;
mod status_banner;
mod toolbar;

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use content::InstalledPackageContent;
use selection::RemoteSelection;
use toolbar::build_toolbar_actions;

use crate::commands::{
    self, AUTOSYNC_PAUSED_EVENT, PACKAGE_STATUS_EVENT, PackageStatusEvent, PausedEvent,
};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Spinner};
use crate::tauri as tauri_bridge;

/// True iff `event` reports an observation the page has not acted on yet — its
/// fingerprint differs from the last one the page refetched for. A tick that
/// re-reports an unchanged tree carries the same fingerprint, so this returns
/// false and the page holds still (keeps its selection, filters, open dialog)
/// instead of tearing down and rebuilding to the identical view.
fn is_new_observation(last_acted: Option<&str>, event: &PackageStatusEvent) -> bool {
    last_acted != Some(event.fingerprint.as_str())
}

// ── Installed Package page ──

/// Which autosync pause reasons warrant the detail page's dedicated paused
/// banner. Diverged / Behind / Ahead are already covered by the status-driven
/// banner, so only the reasons the status string cannot fully convey get the
/// dedicated banner: free-form `"other"` refusals, `"pullConflict"` (which
/// the status string flattens to `"paused"`, hiding the conflict details and
/// the merge-page remediation), and `"roleDenied"` (flattened the same way,
/// hiding both the role and the fact that switching role is the only fix).
fn warrants_paused_banner(reason: &str) -> bool {
    matches!(reason, "other" | "pullConflict" | "roleDenied")
}

#[component]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
pub fn InstalledPackage() -> impl IntoView {
    let query = use_query_map();

    // Version-mismatch banner inputs from the deep-link navigation (Task 5).
    // The requested revision's own remote (bucket + catalog) travels alongside
    // the hash so its message is fetched from where it actually lives.
    let mismatch_requested = query.read_untracked().get("mismatch");
    let mismatch_bucket = query.read_untracked().get("mrbucket");
    let mismatch_catalog = query.read_untracked().get("mrcatalog");
    let local_only = query.read_untracked().get("localOnly").is_some();

    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);
    let refetch = Trigger::new();
    let show_set_remote_popup = RwSignal::new(false);

    let data = LocalResource::new(move || {
        refetch.track();
        let namespace = query.read().get("namespace").unwrap_or_default();
        let filter = query.read().get("filter");
        async move { commands::get_installed_package_data(namespace, filter).await }
    });

    // The remote-file selection is held *here*, not in the content component
    // that renders the checkboxes. `InstalledPackageContent` receives its data
    // by value out of the resource's `Suspend`, so every re-resolution re-runs
    // it and every signal created inside it is a brand-new signal — a selection
    // created down there is destroyed by any refresh. This is the same reason
    // `last_fingerprint` below sits up here.
    let selection = RwSignal::new(RemoteSelection::default());

    // Moving between packages must not carry the previous package's picks over.
    // The router keeps this component **mounted** when only the `namespace`
    // query changes, so nothing unmounts the selection; and because it is keyed
    // by path, a carried-over set would tick same-named files in the package
    // just opened. Compares against the previous value rather than firing on
    // every read, so the first run (which has no previous namespace) is inert.
    Effect::new(move |previous: Option<Option<String>>| {
        let namespace = query.read().get("namespace");
        if previous.is_some_and(|prev| prev != namespace) {
            selection.set(RemoteSelection::default());
        }
        namespace
    });

    // Autosync watcher → page refresh: when the backend reports a
    // status change for the currently-open namespace, refetch the
    // detail data so the entries list and toolbar reflect the new
    // upstream state. Detail data is heavier than the row-level
    // signals on the list page, so we use a full refetch rather than
    // mutate sub-signals individually.
    let event_holder: RwSignal<Option<PackageStatusEvent>> = RwSignal::new(None);
    let listener = tauri_bridge::listen::<PackageStatusEvent>(PACKAGE_STATUS_EVENT, move |ev| {
        event_holder.set(Some(ev));
    });
    on_cleanup(move || drop(listener));

    // Autosync pause event for the currently-open namespace: drives the
    // dedicated paused banner. We only render this banner for the reasons
    // the status string can't convey (`other`, `pullConflict` — see
    // `warrants_paused_banner`) — the regular status banner (`"diverged"`,
    // `"behind"`, `"ahead"`) already conveys the per-state-machine reasons,
    // and stacking the autosync paused banner on top would double up the
    // same information (this was a Greptile finding on the
    // get_autosync_snapshot hydration). Filtering at both ingress
    // points — the live listener AND the snapshot replay — keeps the
    // detail page from showing two banners side-by-side for those.
    let paused_event: RwSignal<Option<PausedEvent>> = RwSignal::new(None);
    // Register the listener BEFORE fetching the snapshot so a pause
    // event that fires between the two doesn't get dropped. If the
    // listener wins the race the snapshot won't overwrite a fresher
    // value — see the `slot.is_none()` check on the seed below.
    let paused_listener = tauri_bridge::listen::<PausedEvent>(AUTOSYNC_PAUSED_EVENT, move |ev| {
        if !warrants_paused_banner(&ev.reason) {
            return;
        }
        let current = query.read_untracked().get("namespace").unwrap_or_default();
        if ev.namespace == current {
            paused_event.set(Some(ev));
        }
    });
    on_cleanup(move || drop(paused_listener));

    // Re-hydrate the paused banner on page mount: the watcher may have
    // paused our namespace before this page existed, in which case the
    // listener above will never fire for that pause. Fetch the
    // watcher's current paused map and seed `paused_event` if our
    // namespace appears with a reason that warrants the dedicated
    // banner (`other` or `pullConflict`).
    leptos::task::spawn_local(async move {
        let Ok(snapshot) = commands::get_autosync_snapshot().await else {
            return;
        };
        let current = query.read_untracked().get("namespace").unwrap_or_default();
        if let Some(entry) = snapshot
            .paused
            .into_iter()
            .find(|p| p.namespace == current && warrants_paused_banner(&p.reason))
        {
            // Don't overwrite a fresher value the live listener may have
            // already set between listener registration and now.
            paused_event.update(|slot| {
                if slot.is_none() {
                    *slot = Some(entry);
                }
            });
        }
    });

    // The last observation this page acted on. A background tick re-reports a
    // package's status every iteration whether or not anything moved; each
    // event carries a fingerprint of the observation, and the page refetches
    // only when it differs from this. A no-op re-report is skipped, so the page
    // keeps its selection, filters, and open dialog instead of rebuilding to
    // the identical view. Read untracked: this effect must run on new events,
    // not when it records its own last-acted value.
    let last_fingerprint: RwSignal<Option<String>> = RwSignal::new(None);
    Effect::new(move |_| {
        let Some(ev) = event_holder.get() else { return };
        let current = query.read().get("namespace").unwrap_or_default();
        if ev.namespace != current {
            return;
        }
        if !is_new_observation(last_fingerprint.get_untracked().as_deref(), &ev) {
            return;
        }
        last_fingerprint.set(Some(ev.fingerprint.clone()));
        // Any status emit other than `"paused"` for this namespace means the
        // watcher has progressed past the pause (or the user manually cleared
        // it via Publish / Pull / Set Remote). Drop the cached message so the
        // banner reverts.
        if ev.status != "paused" {
            paused_event.set(None);
        }
        refetch.notify();
    });

    // A `Transition` (not `Suspense`) is deliberate: a plain `Suspense` shows its
    // fallback whenever the resource re-enters a pending state, so every refresh
    // that survives the fingerprint gate blanks the whole screen to a spinner and
    // back — for a change that may have nothing to do with what is on it.
    // `Transition` keeps the already-rendered children mounted while a later load
    // is pending and falls back only on the initial one, so a genuine change
    // updates the page in place. The commit screen made the same switch for a
    // related reason (see the note on its boundary).
    view! {
        <Transition fallback=move || {
            view! {
                <Layout breadcrumbs=vec![] notification=notification ui_locked=ui_locked>
                    <Spinner />
                </Layout>
            }
        }>
            {move || {
                let mismatch_requested = mismatch_requested.clone();
                let mismatch_bucket = mismatch_bucket.clone();
                let mismatch_catalog = mismatch_catalog.clone();
                Suspend::new(async move {
                    match data.await {
                        Ok(d) => {
                            let ns = d.namespace.clone();
                            let breadcrumbs = vec![
                                BreadcrumbItem::Link(BreadcrumbLink {
                                    href: "/installed-packages-list".to_string(),
                                    title: String::new(),
                                }),
                                BreadcrumbItem::Current(ns),
                            ];
                            let actions = build_toolbar_actions(
                                &d,
                                notification,
                                ui_locked,
                                show_set_remote_popup,
                            );
                            view! {
                                <Layout breadcrumbs=breadcrumbs notification=notification actions=actions ui_locked=ui_locked>
                                    <InstalledPackageContent
                                        data=d
                                        notification=notification
                                        ui_locked=ui_locked
                                        refetch=refetch
                                        mismatch_requested=mismatch_requested.clone()
                                        mismatch_bucket=mismatch_bucket.clone()
                                        mismatch_catalog=mismatch_catalog.clone()
                                        local_only=local_only
                                        show_set_remote_popup=show_set_remote_popup
                                        paused_event=paused_event
                                        selection=selection
                                    />
                                </Layout>
                            }
                                .into_any()
                        }
                        Err(e) => {
                            crate::error_handler::handle_or_display(&e, notification)
                        }
                    }
                })
            }}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::warrants_paused_banner;

    #[test]
    fn message_bearing_reasons_get_the_dedicated_banner() {
        assert!(warrants_paused_banner("other"));
        assert!(warrants_paused_banner("pullConflict"));
        // The status string flattens a denial to "paused", which tells the
        // user nothing about the role or the one action that clears it.
        assert!(warrants_paused_banner("roleDenied"));
    }

    #[test]
    fn status_legible_reasons_are_filtered_out() {
        assert!(!warrants_paused_banner("diverged"));
        assert!(!warrants_paused_banner("behind"));
        assert!(!warrants_paused_banner("pendingChanges"));
        assert!(!warrants_paused_banner("pendingCommit"));
    }

    // NOTE: `#[wasm_bindgen_test]`, not `#[test]` — the wasm runner never
    // collects a plain `#[test]` (the ones above compile but do not run).
    use super::{PackageStatusEvent, is_new_observation};
    use wasm_bindgen_test::wasm_bindgen_test;

    fn ev(fingerprint: &str) -> PackageStatusEvent {
        PackageStatusEvent {
            namespace: "acme/demo".to_string(),
            status: "up_to_date".to_string(),
            has_changes: false,
            fingerprint: fingerprint.to_string(),
        }
    }

    #[wasm_bindgen_test]
    fn first_event_is_a_new_observation() {
        // Nothing acted on yet — the first event always refetches.
        assert!(is_new_observation(None, &ev("up_to_date;")));
    }

    #[wasm_bindgen_test]
    fn same_fingerprint_holds_still() {
        // A no-op tick re-reports the same observation — the page must not act.
        assert!(!is_new_observation(Some("up_to_date;"), &ev("up_to_date;")));
    }

    #[wasm_bindgen_test]
    fn changed_fingerprint_is_a_new_observation() {
        // A real change (a modified path enters the digest) → refetch.
        assert!(is_new_observation(
            Some("up_to_date;"),
            &ev("up_to_date;a.txt:M:h;")
        ));
    }
}
