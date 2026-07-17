mod content;
mod entries;
mod status_banner;
mod toolbar;

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use content::InstalledPackageContent;
use toolbar::build_toolbar_actions;

use crate::commands::{
    self, AUTOSYNC_PAUSED_EVENT, PACKAGE_STATUS_EVENT, PackageStatusEvent, PausedEvent,
};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Spinner};
use crate::tauri as tauri_bridge;

// ── Installed Package page ──

#[component]
pub fn InstalledPackage() -> impl IntoView {
    let query = use_query_map();

    // Version-mismatch banner inputs from the deep-link navigation (Task 5).
    let mismatch_requested = query.read_untracked().get("mismatch");
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
    // dedicated paused banner. We only render this banner for `Other(_)`
    // pauses — the regular status banner (`"diverged"`, `"behind"`,
    // `"ahead"`) already conveys the per-state-machine reasons, and
    // stacking the autosync paused banner on top would double up the
    // same information (this was a Greptile finding on the
    // get_autosync_snapshot hydration). Filtering at both ingress
    // points — the live listener AND the snapshot replay — keeps the
    // detail page from showing two banners side-by-side for any
    // non-Other paused namespace.
    let paused_event: RwSignal<Option<PausedEvent>> = RwSignal::new(None);
    // Register the listener BEFORE fetching the snapshot so a pause
    // event that fires between the two doesn't get dropped. If the
    // listener wins the race the snapshot won't overwrite a fresher
    // value — see the `slot.is_none()` check on the seed below.
    let paused_listener = tauri_bridge::listen::<PausedEvent>(AUTOSYNC_PAUSED_EVENT, move |ev| {
        if ev.reason != "other" {
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
    // banner (i.e. `other`).
    leptos::task::spawn_local(async move {
        let Ok(snapshot) = commands::get_autosync_snapshot().await else {
            return;
        };
        let current = query.read_untracked().get("namespace").unwrap_or_default();
        if let Some(entry) = snapshot
            .paused
            .into_iter()
            .find(|p| p.namespace == current && p.reason == "other")
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

    Effect::new(move |_| {
        let Some(ev) = event_holder.get() else { return };
        let current = query.read().get("namespace").unwrap_or_default();
        if ev.namespace == current {
            // Any status emit other than `"paused"` for this namespace
            // means the watcher has progressed past the pause (or the
            // user manually cleared it via Publish / Pull / Set Remote).
            // Drop the cached message so the banner reverts.
            if ev.status != "paused" {
                paused_event.set(None);
            }
            refetch.notify();
        }
    });

    view! {
        <Suspense fallback=move || {
            view! {
                <Layout breadcrumbs=vec![] notification=notification ui_locked=ui_locked>
                    <Spinner />
                </Layout>
            }
        }>
            {move || {
                let mismatch_requested = mismatch_requested.clone();
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
                                        local_only=local_only
                                        show_set_remote_popup=show_set_remote_popup
                                        paused_event=paused_event
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
        </Suspense>
    }
}
