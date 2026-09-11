//! §3.2's feed: one row per file, flat, in the order the backend sent it. The
//! ordering and the cap are the backend's (§4.5) and are not re-derived here —
//! two sources of truth for "newest" is how they drift.
//!
//! A file row carries no state, no tone and no `provisional`: a path and an
//! mtime are facts on disk. That is why this view was buildable before the
//! status scales were settled, and why nothing here imports [`crate::kit::render`].
//!
//! Flat is the default; an optional `Package` axis groups the same rows under
//! a heading per namespace without touching the order within a group.

use std::collections::BTreeMap;

use leptos::prelude::*;

use crate::commands;
use crate::commands::MainPageFileData;
use crate::kit::Blankslate;
use crate::kit::Card;
use crate::kit::FileRow;
use crate::kit::GroupHeading;

#[cfg(test)]
use super::GROUP_NONE;
use super::GROUP_PACKAGE;
use super::package_page_href;

/// The card, on one payload — no title, because the list toolbar's toggle above
/// it names the view; see `Card::title`'s own doc for why. Draws flat by
/// default; `group_by` can switch it to one heading per package instead.
#[component]
pub fn RecentFilesRegion(
    files: Vec<MainPageFileData>,
    /// R2's search reaches this view too, on `path` (R5). A `Signal` rather
    /// than a plain `String` keeps this region a pure view of what it is
    /// handed — it reads the signal, it never writes it — so a keystroke
    /// narrows the feed without rebuilding this component.
    query: Signal<String>,
    /// The feed's own axis (`GROUP_NONE`, the default, or `GROUP_PACKAGE`).
    /// A `Signal`, matching `query` above and for the identical reason: this
    /// region only reads the axis it is handed, never writes it.
    group_by: Signal<String>,
) -> impl IntoView {
    if files.is_empty() {
        return view! {
            <Card>
                <Blankslate
                    heading="No files yet"
                    description="Files appear here once you install a package or publish a change."
                />
            </Card>
        }
        .into_any();
    }

    view! {
        <Card>
            {move || {
                let text = query.get();
                let needle = text.trim().to_lowercase();
                let visible: Vec<MainPageFileData> = if needle.is_empty() {
                    files.clone()
                } else {
                    files
                        .clone()
                        .into_iter()
                        .filter(|f| f.path.to_lowercase().contains(&needle))
                        .collect()
                };
                if visible.is_empty() {
                    view! {
                        <Blankslate
                            heading=format!("No files match \u{201c}{}\u{201d}", text.trim())
                            description="Search covers the paths of files you have locally. Files that exist \
                                  only in a bucket are not included."
                        />
                    }
                        .into_any()
                } else if group_by.get() == GROUP_PACKAGE {
                    // `BTreeMap` gives the alphabetical group order for free and,
                    // because `visible` is pushed into each entry in the order it
                    // arrives, preserves the wire order within a group too — the
                    // backend's order (§4.5) is not re-derived here.
                    let mut by_namespace: BTreeMap<String, Vec<MainPageFileData>> = BTreeMap::new();
                    for f in visible {
                        by_namespace.entry(f.namespace.clone()).or_default().push(f);
                    }
                    by_namespace
                        .into_iter()
                        .map(|(namespace, group_files)| {
                            // Never written by hand — the count is the length of
                            // the rows that follow, always, including one.
                            let count = group_files.len();
                            view! {
                                <GroupHeading title=namespace count=count />
                                {group_files.iter().map(file_row).collect_view()}
                            }
                        })
                        .collect_view()
                        .into_any()
                } else {
                    visible.iter().map(file_row).collect_view().into_any()
                }
            }}
        </Card>
    }
    .into_any()
}

/// One file, wherever it is drawn — a flat row or a row under a `GroupHeading`.
/// Pulled out so the two call sites in [`RecentFilesRegion`] draw the identical
/// row rather than two copies of the same closures drifting apart.
fn file_row(f: &MainPageFileData) -> impl IntoView + use<> {
    let namespace = f.namespace.clone();
    let open_ns = namespace.clone();
    let open_path = f.path.clone();
    let reveal_ns = namespace.clone();
    let reveal_path = f.path.clone();
    view! {
        <FileRow
            path=f.path.clone()
            package=namespace.clone()
            package_href=package_page_href(&namespace)
            at=f.changed_at
            on_open=move |_| {
                let (ns, path) = (open_ns.clone(), open_path.clone());
                leptos::task::spawn_local(async move {
                    if let Err(err) = commands::open_in_default_application(ns, path, None).await {
                        // Logged, never rendered: the words a
                        // user reads come only from the kit.
                        web_sys::console::error_1(
                            &format!("open_in_default_application failed: {err}").into(),
                        );
                    }
                });
            }
            on_reveal=move |_| {
                let (ns, path) = (reveal_ns.clone(), reveal_path.clone());
                leptos::task::spawn_local(async move {
                    if let Err(err) = commands::reveal_in_file_browser(ns, path, None).await {
                        web_sys::console::error_1(
                            &format!("reveal_in_file_browser failed: {err}").into(),
                        );
                    }
                });
            }
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// `main_page.rs`'s pattern.
    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    fn file(path: &str, namespace: &str, changed_at: f64) -> MainPageFileData {
        MainPageFileData {
            path: path.to_string(),
            namespace: namespace.to_string(),
            changed_at,
        }
    }

    /// Inside a `Router`: `FileRow`'s package tag is an `<a>` rather than a
    /// `use_navigate` call, but mount inside one anyway to match this file's
    /// siblings and to stay safe if that ever changes.
    fn mount_feed(files: Vec<MainPageFileData>) -> web_sys::Element {
        mount_feed_grouped(files, GROUP_NONE)
    }

    /// `mount_feed`, with the axis under test named explicitly.
    fn mount_feed_grouped(files: Vec<MainPageFileData>, axis: &str) -> web_sys::Element {
        let axis = axis.to_string();
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <RecentFilesRegion
                        files=files
                        query=Signal::stored(String::new())
                        group_by=Signal::stored(axis)
                    />
                </leptos_router::components::Router>
            }
        })
    }

    #[wasm_bindgen_test]
    fn the_feed_lists_every_file_in_the_order_the_backend_sent() {
        // The backend sorts and caps (§4.5). The UI must not re-sort — a second
        // ordering here would be a second source of truth for "newest".
        //
        // Non-monotonic in `changed_at` (middle, then high, then low) on purpose:
        // a strictly descending fixture cannot tell "no re-sort" apart from "a
        // redundant re-sort in the same, already-correct direction" — with two
        // elements any order is trivially consistent with either sort direction.
        // This order is consistent with neither, so ANY re-sort by `changed_at`,
        // ascending or descending, reorders these rows and fails the assertion.
        let el = mount_feed(vec![
            file("b/two.csv", "user/beta", 5_000.0),
            file("c/three.csv", "user/gamma", 9_000.0),
            file("a/one.csv", "user/alpha", 1_000.0),
        ]);
        let text = el.text_content().unwrap();
        let two = text.find("b/two.csv").expect("first row");
        let three = text.find("c/three.csv").expect("second row");
        let one = text.find("a/one.csv").expect("third row");
        assert!(
            two < three && three < one,
            "the wire's order is the screen's order: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn a_file_row_links_to_its_owning_package() {
        let el = mount_feed(vec![file("a/one.csv", "user/alpha", 1_000.0)]);
        let href = el
            .query_selector("a[href*=installed-package]")
            .unwrap()
            .expect("the package tag is a link")
            .get_attribute("href")
            .unwrap();
        // The same bug `a_row_links_to_its_own_package` pins for the packages view:
        // the package page reads its namespace from the query string.
        assert!(
            href.contains("namespace=user/alpha"),
            "href must carry the namespace, got: {href}"
        );
    }

    #[wasm_bindgen_test]
    fn an_empty_feed_is_a_blankslate_and_never_an_empty_card() {
        // A fresh install has no files. An empty Card with a title and nothing in it
        // says less than a sentence does.
        let el = mount_feed(vec![]);
        let text = el.text_content().unwrap();
        assert!(text.contains("No files yet"), "got: {text}");
        assert!(
            text.contains("install a package or publish a change"),
            "got: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn the_feed_draws_no_state_label_at_all() {
        // The components doc's reason this view was buildable first: a file row
        // carries no state. If a `StateLabel` ever appears here, someone has
        // imported `render` and the view has grown a second job.
        let el = mount_feed(vec![file("a/one.csv", "user/alpha", 1_000.0)]);
        let text = el.text_content().unwrap();
        for word in ["Latest", "Not the latest", "files changed", "Sync stopped"] {
            assert!(
                !text.contains(word),
                "a file row says nothing about state: {text}"
            );
        }
    }

    #[wasm_bindgen_test]
    fn grouping_the_feed_by_package_keeps_the_wire_order_inside_each_group() {
        // §4.5's cap and order are the backend's. Grouping partitions them; a
        // re-sort inside a group would be the second source of truth for "newest"
        // that the flat view's own test exists to forbid.
        //
        // `user/beta`'s three files, in wire order, are 5_000 then 1_000 then
        // 9_000 — non-monotonic in BOTH directions. Two files are not enough:
        // 5_000-then-1_000 alone is already descending, so a group re-sorted
        // newest-first would leave that pair untouched and the test would pass
        // for the wrong reason. The third file makes every re-sort, ascending
        // or descending, reorder these three and fail the assertion below.
        let el = mount_feed_grouped(
            vec![
                file("b/mid.csv", "user/beta", 5_000.0),
                file("a/new.csv", "user/alpha", 9_000.0),
                file("b/old.csv", "user/beta", 1_000.0),
                file("b/new.csv", "user/beta", 9_000.0),
            ],
            GROUP_PACKAGE,
        );

        let text = el.text_content().unwrap();
        let mid = text.find("b/mid.csv").expect("mid");
        let old = text.find("b/old.csv").expect("old");
        let new = text.find("b/new.csv").expect("new");
        assert!(
            mid < old && old < new,
            "the wire's order survives inside the group: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn the_feed_groups_are_alphabetical_by_package() {
        let el = mount_feed_grouped(
            vec![
                file("z/one.csv", "user/zebra", 9_000.0),
                file("a/two.csv", "user/apple", 1_000.0),
            ],
            GROUP_PACKAGE,
        );

        let text = el.text_content().unwrap();
        let apple = text.find("user/apple").expect("apple");
        let zebra = text.find("user/zebra").expect("zebra");
        assert!(
            apple < zebra,
            "groups are alphabetical, NOT ordered by their newest file — that is \
             the arrangement §3.2 rejects: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn the_feed_draws_no_headings_on_the_none_axis() {
        // The default, and what plan 7 shipped. A heading here would be a
        // regression against §3.2's flat default.
        //
        // `[class*=group]` matches nothing in this crate: grepping the generated
        // `assets/css/kit/_modules.scss` shows stylance emits `<class>-<hash>`
        // (`root-e6e1377`, `title-e6e1377`, `annotation-e6e1377`, `count-e6e1377`
        // for `group_heading`), never module-prefixed, so a selector containing
        // "group" would pass here for the wrong reason and prove nothing.
        //
        // `GroupHeading` is the only kit component that renders its own title in
        // a `<span>` — `Card` (`card.rs:38`) and `Dialog` (`dialog.rs:73`) both
        // use `<h2>` — so `span[class*=title]` identifies a group heading and
        // nothing else within the feed's subtree.
        let none_el =
            mount_feed_grouped(vec![file("a/one.csv", "user/alpha", 1_000.0)], GROUP_NONE);
        assert!(
            none_el
                .query_selector("span[class*=title]")
                .unwrap()
                .is_none(),
            "no headings on the flat axis"
        );

        // Paired with the identical selector on the grouped axis: without this,
        // a selector matching nothing would pass the assertion above even
        // against an implementation that drew a heading on every axis — the
        // exact failure this task's brief calls out.
        let grouped_el = mount_feed_grouped(
            vec![file("a/one.csv", "user/alpha", 1_000.0)],
            GROUP_PACKAGE,
        );
        assert!(
            grouped_el
                .query_selector("span[class*=title]")
                .unwrap()
                .is_some(),
            "a heading is drawn on the package axis"
        );
    }
}
