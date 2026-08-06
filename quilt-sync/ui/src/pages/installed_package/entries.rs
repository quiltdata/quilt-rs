use std::collections::BTreeSet;

use leptos::prelude::*;

use quilt_uri::S3PackageUri;

use super::selection::{RemoteSelection, toggled_path};
use crate::commands::{self, EntryData};
use crate::components::buttons;
use crate::components::{IgnorePopupData, Notification, UnignorePopupData};
use crate::util;
use crate::util::format_size;

// ── Entries toolbar ──

#[component]
#[allow(
    clippy::fn_params_excessive_bools,
    reason = "Leptos props are named at every call site, so the mis-ordering \
              this lint guards against cannot happen here"
)]
pub(super) fn EntriesToolbar(
    /// Whether this package is on whole-package scope. It changes what the
    /// left slot holds, and the slot is never left empty: select-all plus the
    /// download control under the narrow scope, a whole-package download while
    /// files are still pending, and the standing-scope line once they are not.
    whole_package: bool,
    /// What that slot says when whole-package scope has nothing left to fetch.
    standing_line: &'static str,
    has_remote_entries: bool,
    on_select_all: impl Fn(leptos::ev::Event) + 'static,
    all_selected: Memo<bool>,
    /// Some remote entries are ticked but not all — draws the box indeterminate
    /// rather than empty, which would read as "nothing is selected".
    partially_selected: Memo<bool>,
    checked_count: Memo<usize>,
    on_install_paths: impl Fn(leptos::ev::MouseEvent) + 'static,
    filter_unmodified: RwSignal<bool>,
    filter_ignored: RwSignal<bool>,
    ignored_count: usize,
    unmodified_count: usize,
    with_status: bool,
    /// Whether the sync-scope band is rendered above this toolbar. Both are
    /// sticky, so the one underneath has to stick lower or the two overlap.
    below_sync_scope: bool,
) -> impl IntoView {
    let toolbar_class = match (with_status, below_sync_scope) {
        (true, true) => "qui-entries-toolbar with-status below-sync-scope",
        (true, false) => "qui-entries-toolbar with-status",
        (false, true) => "qui-entries-toolbar below-sync-scope",
        (false, false) => "qui-entries-toolbar",
    };

    view! {
        <div class=toolbar_class>
            <div class="container">
                {if whole_package && !has_remote_entries {
                    // Caught up. The list below already shows what is on disk,
                    // so this states the only thing it cannot: later arrivals
                    // come too. It also keeps the slot from going empty.
                    view! {
                        <span class="value default scope-standing-line">{standing_line}</span>
                    }
                        .into_any()
                } else if has_remote_entries {
                    {
                        let install_btn_class = Memo::new(move |_| {
                            if checked_count.get() > 0 {
                                "qui-button primary"
                            } else {
                                "qui-button"
                            }
                        });
                        view! {
                            // Under whole-package scope there is no per-file
                            // choice left to offer, so select-all goes rather
                            // than sitting inert. The button stays: it is the
                            // catch-up affordance, and its existing
                            // show-when-pending rule already renders it exactly
                            // when it is the right thing to press.
                            {(!whole_package).then(|| view! {
                                <label class="select-all">
                                    <input
                                        type="checkbox"
                                        prop:checked=move || all_selected.get()
                                        prop:indeterminate=move || partially_selected.get()
                                        on:change=on_select_all
                                    />
                                    "Select all"
                                </label>
                            })}
                            <button
                                class=move || install_btn_class.get()
                                type="button"
                                prop:disabled=move || checked_count.get() == 0
                                on:click=on_install_paths
                            >
                                <span>
                                    {if whole_package {
                                        "Download all files"
                                    } else {
                                        "Download selected paths"
                                    }}
                                </span>
                            </button>
                        }.into_any()
                    }
                } else {
                    ().into_any()
                }}
                <EntriesFilter
                    filter_unmodified=filter_unmodified
                    filter_ignored=filter_ignored
                    ignored_count=ignored_count
                    unmodified_count=unmodified_count
                />
            </div>
        </div>
    }
}

// ── Entries filter ──

#[component]
fn EntriesFilter(
    filter_unmodified: RwSignal<bool>,
    filter_ignored: RwSignal<bool>,
    ignored_count: usize,
    unmodified_count: usize,
) -> impl IntoView {
    let show_filter = ignored_count > 0 || unmodified_count > 0;

    view! {
        <Show when=move || show_filter>
            <div class="filter">
                <div class="qui-entries-filter">
                    <span>"Show"</span>
                    <label>
                        <input
                            type="checkbox"
                            prop:checked=move || filter_unmodified.get()
                            on:change=move |_| {
                                filter_unmodified.set(!filter_unmodified.get_untracked());
                            }
                        />
                        "unmodified"
                        <Show when=move || !filter_unmodified.get() && (unmodified_count > 0)>
                            <span class="qui-filter-count">
                                {format!("({unmodified_count})")}
                            </span>
                        </Show>
                    </label>
                    <label>
                        <input
                            type="checkbox"
                            prop:checked=move || filter_ignored.get()
                            on:change=move |_| {
                                filter_ignored.set(!filter_ignored.get_untracked());
                            }
                        />
                        "ignored"
                        <Show when=move || !filter_ignored.get() && (ignored_count > 0)>
                            <span class="qui-filter-count">
                                {format!("({ignored_count})")}
                            </span>
                        </Show>
                    </label>
                </div>
            </div>
        </Show>
    }
}

// ── Entry row ──

#[component]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
pub(super) fn EntryRow(
    /// Under whole-package scope the row checkbox is inert. Forced, not
    /// inferred: relying on "a downloaded file disables its own" would leave
    /// rows live during the initial catch-up and after a failed one — offering
    /// a per-file choice the scope has just removed, at exactly the moment
    /// someone is watching.
    whole_package: bool,
    entry: EntryData,
    pkg_uri: Option<S3PackageUri>,
    /// The held selection, written on a checkbox click.
    selection: RwSignal<RemoteSelection>,
    /// What is ticked right now — the one derivation the header shares, read
    /// rather than mirrored, so a row cannot disagree with it.
    selected: Memo<BTreeSet<String>>,
    /// The remote entries the package currently offers, for resolving a toggle.
    remote_paths: StoredValue<BTreeSet<String>>,
    notification: RwSignal<Option<Notification>>,
    show_ignore_popup: RwSignal<Option<IgnorePopupData>>,
    show_unignore_popup: RwSignal<Option<UnignorePopupData>>,
) -> impl IntoView {
    let EntryData {
        filename,
        size,
        status,
        junky_pattern,
        ignored_by,
        namespace,
    } = entry;

    let is_remote = status == "remote";
    let is_deleted = status == "deleted";
    let is_ignored = ignored_by.is_some();
    let is_junky = junky_pattern.is_some();

    let class_mods = {
        let mut classes = vec![status.as_str()];
        if is_junky {
            classes.push("junky");
        }
        if is_ignored {
            classes.push("ignored");
        }
        format!("qui-entry {}", classes.join(" "))
    };

    let status_display = match status.as_str() {
        "added" => "New",
        "deleted" => "Deleted",
        "modified" => "Modified",
        "pristine" => "Downloaded",
        "remote" => "Remote",
        _ => "",
    };

    let size_display = format_size(size);
    let status_text = format!("{status_display}, {size_display}");

    let filename_display = filename.clone();
    let filename_title = filename.clone();

    // Checkbox state for remote entries
    let name_for_check = filename.clone();
    let is_checked = Memo::new(move |_| {
        if !is_remote {
            return true; // non-remote always show as checked (disabled)
        }
        selected.with(|s| s.contains(&name_for_check))
    });

    let name_for_toggle = filename.clone();
    let on_checkbox_change = move |_| {
        if !is_remote {
            return;
        }
        let current = selection.get_untracked();
        selection.set(
            remote_paths.with_value(|remote| toggled_path(&current, remote, &name_for_toggle)),
        );
    };

    // Action buttons
    let show_open_reveal = !is_remote && !is_deleted && !is_ignored;
    let show_catalog = (is_remote || status == "pristine")
        && pkg_uri.as_ref().is_some_and(|u| u.catalog.is_some());

    let ns_for_open = namespace.clone();
    let path_for_open = filename.clone();
    let uri_for_open = pkg_uri.clone();
    let uri_for_ignore = pkg_uri.clone();
    let uri_for_unignore = pkg_uri.clone();
    let on_open = move |_| {
        let ns = ns_for_open.clone();
        let path = path_for_open.clone();
        let uri = uri_for_open.clone();
        let notification = notification;
        leptos::task::spawn_local(async move {
            match commands::open_in_default_application(ns, path, uri).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    let ns_for_reveal = namespace.clone();
    let path_for_reveal = filename.clone();
    let uri_for_reveal = pkg_uri.clone();
    let on_reveal = move |_| {
        let ns = ns_for_reveal.clone();
        let path = path_for_reveal.clone();
        let uri = uri_for_reveal.clone();
        let notification = notification;
        leptos::task::spawn_local(async move {
            match commands::reveal_in_file_browser(ns, path, uri).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    let path_for_catalog = filename.clone();
    let on_open_catalog = move |_| {
        let Some(url) = pkg_uri
            .as_ref()
            .and_then(|u| util::entry_catalog_url(u, &path_for_catalog))
        else {
            return;
        };
        leptos::task::spawn_local(async move {
            let _ = commands::open_in_web_browser(url).await;
        });
    };

    let ns_for_ignore = namespace.clone();
    let path_for_ignore = filename;
    let on_ignore = move |_| {
        if let Some(pattern) = junky_pattern.clone() {
            show_ignore_popup.set(Some(IgnorePopupData {
                namespace: ns_for_ignore.clone(),
                path: path_for_ignore.clone(),
                suggested_pattern: pattern,
                uri: uri_for_ignore.clone(),
            }));
        }
    };

    let ns_for_unignore = namespace;
    let on_unignore = move |_| {
        if let Some(pattern) = ignored_by.clone() {
            show_unignore_popup.set(Some(UnignorePopupData {
                namespace: ns_for_unignore.clone(),
                pattern,
                uri: uri_for_unignore.clone(),
            }));
        }
    };

    view! {
        <div class=class_mods>
            <label class="avatar">
                <input
                    type="checkbox"
                    prop:checked=move || is_checked.get()
                    prop:disabled=whole_package || !is_remote
                    on:change=on_checkbox_change
                />
            </label>

            <div class="text">
                <p class="text-primary" title=filename_title data-testid="entry-name">
                    {filename_display}
                </p>
                <p class="text-secondary">{status_text}</p>
            </div>

            <div class="menu">
                <ul class="menu-list">
                    {if show_open_reveal {
                        view! {
                            <li class="menu-item">
                                <buttons::Open on_click=on_open small=true />
                            </li>
                            <li class="menu-item">
                                <buttons::Reveal on_click=on_reveal small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {if show_catalog {
                        view! {
                            <li class="menu-item">
                                <buttons::OpenInCatalog on_click=on_open_catalog small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {if is_junky {
                        view! {
                            <li class="menu-item">
                                <buttons::Ignore on_click=on_ignore small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {if is_ignored {
                        view! {
                            <li class="menu-item">
                                <buttons::Unignore on_click=on_unignore small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                </ul>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::EntriesToolbar;
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// The toolbar's header checkbox in one selection state. `indeterminate` is a
    /// DOM *property* with no attribute form, so it can only be checked against a
    /// real element — which is the whole reason these two tests are here rather
    /// than beside the pure rules in `super::super::selection`.
    fn header_checkbox(all: bool, partial: bool) -> web_sys::HtmlInputElement {
        let el = mount(move || {
            view! {
                <EntriesToolbar
                    below_sync_scope=false
                    whole_package=false
                    standing_line="Files added later are downloaded too."
                    has_remote_entries=true
                    on_select_all=|_| {}
                    all_selected=Memo::new(move |_| all)
                    partially_selected=Memo::new(move |_| partial)
                    checked_count=Memo::new(move |_| usize::from(all || partial))
                    on_install_paths=|_| {}
                    filter_unmodified=RwSignal::new(true)
                    filter_ignored=RwSignal::new(true)
                    ignored_count=0
                    unmodified_count=0
                    with_status=false
                />
            }
        });
        el.query_selector(".select-all input")
            .unwrap()
            .expect("the toolbar still renders a select-all checkbox")
            .dyn_into()
            .unwrap()
    }

    /// A partial selection draws **indeterminate**, not empty. An empty box says
    /// "nothing is selected", which was a momentary lie while the selection died
    /// on every refresh and is a standing one now that it survives.
    #[wasm_bindgen_test]
    fn a_partial_selection_draws_indeterminate() {
        let header = header_checkbox(false, true);
        assert!(header.indeterminate());
        assert!(!header.checked());
    }

    /// The contrast, so the test above cannot pass by everything being drawn
    /// indeterminate: full is plainly checked, empty is plainly empty.
    #[wasm_bindgen_test]
    fn full_and_empty_selections_draw_plainly() {
        let full = header_checkbox(true, false);
        assert!(full.checked());
        assert!(!full.indeterminate());

        let empty = header_checkbox(false, false);
        assert!(!empty.checked());
        assert!(!empty.indeterminate());
    }
}
