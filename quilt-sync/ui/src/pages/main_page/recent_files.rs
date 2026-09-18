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
use std::collections::HashMap;
use std::time::Duration;

use leptos::ev::MouseEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use quilt_uri::S3PackageUri;

use super::super::main_page::rows_class;
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
    /// The `quilt+s3` handle of each package the packages payload named, by
    /// namespace — what a row's Copy action addresses.
    ///
    /// A package with no bucket has no such address, so it is absent here and
    /// its rows draw no Copy button. Absent, not inert: a button that cannot do
    /// its job is worse than no button. The feed's own payload carries no
    /// bucket, which is why this comes from the caller rather than from `files`.
    #[prop(optional)]
    addresses: HashMap<String, S3PackageUri>,
) -> impl IntoView {
    // The one row whose address is on the clipboard, and the only one that may
    // say so. One signal rather than a flag per row: exactly one copy can be the
    // most recent, and the rows read it through their own `copied` prop.
    let copied: RwSignal<Option<Copied>> = RwSignal::new(None);

    if files.is_empty() {
        return view! {
            <Card label="Recent files">
                <Blankslate
                    heading="No files yet"
                    description="Files appear here once you install a package or publish a change."
                />
            </Card>
        }
        .into_any();
    }

    // Filter, then the optional package axis, in one place. A `Memo`: the keyed
    // lists below read a value that is diffed rather than a subtree that is
    // rebuilt (see [`FeedShape`]).
    let arranged = Memo::new(move |_| {
        let needle = query.with(|text| text.trim().to_lowercase());
        let visible: Vec<MainPageFileData> = if needle.is_empty() {
            files.clone()
        } else {
            files
                .iter()
                .filter(|f| f.path.to_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
        if group_by.get() != GROUP_PACKAGE {
            return vec![FileGroup {
                title: None,
                files: visible,
            }];
        }
        // `BTreeMap` gives the alphabetical group order. Pushing in arrival
        // order keeps the backend's newest-first order inside a group, which
        // nothing here re-derives.
        let mut by_namespace: BTreeMap<String, Vec<MainPageFileData>> = BTreeMap::new();
        for f in visible {
            by_namespace.entry(f.namespace.clone()).or_default().push(f);
        }
        by_namespace
            .into_iter()
            .map(|(namespace, files)| FileGroup {
                title: Some(namespace),
                files,
            })
            .collect()
    });
    // The flat axis answers an empty filter with one empty group, so "nothing
    // matched" is every group empty rather than no group. An empty payload
    // returned early above, so nothing reaches here with an empty query.
    let shape = Memo::new(move |_| {
        if arranged.with(|groups| groups.iter().all(|group| group.files.is_empty())) {
            FeedShape::NoMatch
        } else {
            FeedShape::Groups
        }
    });

    view! {
        <Card label="Recent files">
            {move || match shape.get() {
                FeedShape::NoMatch => {
                    // Its own closure: the words carry the query, so they follow
                    // every keystroke while the shape stands still.
                    view! {
                        {move || {
                            let text = query.get();
                            view! {
                                <Blankslate
                                    heading=format!("No files match \u{201c}{}\u{201d}", text.trim())
                                    description="Search covers the paths of files you have locally. Files that exist \
                                          only in a bucket are not included."
                                />
                            }
                        }}
                    }
                        .into_any()
                }
                FeedShape::Groups => {
                    // Cloned per run, not moved: the closure a `Card` body takes
                    // is `FnMut`, and the shape changes whenever a search empties
                    // the feed and fills it again.
                    let addresses = addresses.clone();
                    view! {
                        <For
                            each=move || arranged.get()
                            key=|group: &FileGroup| group.title.clone()
                            children=move |group| {
                                file_group(group.title, arranged, addresses.clone(), copied)
                            }
                        />
                    }
                        .into_any()
                }
            }}
            // The copy's words, for a reader who cannot see the check the row
            // draws. At card level, outside the rows: a region inside a row is
            // replaced rather than updated when the list re-arranges, and a
            // replaced live region never fires. Last in the body, because
            // `Card`'s `.body > * + *` counts an out-of-flow child too and a
            // first-child region would give the list a margin it does not have.
            <span data-sr-only role="status">
                {move || {
                    copied
                        .with(|copied| {
                            copied
                                .as_ref()
                                .map(|copied| format!("Copied the address of {}", copied.path))
                                .unwrap_or_default()
                        })
                }}
            </span>
        </Card>
    }
    .into_any()
}

/// The address that is on the clipboard, and the row it came from.
///
/// The path travels with it for the live region's words alone: "Copied" on its
/// own, announced out of a list of files, does not say which one.
#[derive(Clone, Debug, PartialEq)]
struct Copied {
    uri: String,
    path: String,
}

/// How long a copied row keeps its check.
///
/// Long enough to be seen after the click that caused it, short enough that a
/// reader who comes back to the card later is not told something stale about
/// their clipboard — which by then anything else may own.
const CONFIRM_FOR: Duration = Duration::from_millis(1_500);

/// Put one file's `quilt+s3` address on the clipboard, and mark the row if it
/// lands.
///
/// Only a copy the backend confirmed sets the mark. A row that marked itself on
/// click would be lying in the one case that matters — the clipboard refusing.
fn copy_uri(package: &S3PackageUri, path: &str, copied: RwSignal<Option<Copied>>) {
    let uri = crate::util::file_uri(package, path);
    let text = uri.display();
    let mark = Copied {
        uri: text.clone(),
        path: path.to_string(),
    };
    spawn_local(async move {
        match commands::copy_to_clipboard(text, Some(uri)).await {
            Ok(_) => {
                copied.set(Some(mark.clone()));
                // Cleared only if it is still this row's mark: a second copy in
                // the meantime owns the clipboard, and this timer must not take
                // that newer row's check away with it.
                set_timeout(
                    move || {
                        copied.update(|current| {
                            if current.as_ref() == Some(&mark) {
                                *current = None;
                            }
                        });
                    },
                    CONFIRM_FOR,
                );
            }
            // Logged, never rendered: the words a user reads come only from the
            // kit, and the row simply does not claim the copy happened.
            Err(err) => {
                web_sys::console::error_1(&format!("copy_to_clipboard failed: {err}").into());
            }
        }
    });
}

/// One run of feed rows under one heading — the feed's counterpart to the
/// list's `PackageGroup`. `None` is the flat axis, which draws no heading.
#[derive(Clone, Debug, PartialEq)]
struct FileGroup {
    title: Option<String>,
    files: Vec<MainPageFileData>,
}

/// What the feed draws. Held in a `Memo`, so only a change of shape re-renders
/// the card.
///
/// A reactive closure's `rebuild` discards its whole subtree, so a `<For>` under
/// a closure that re-runs per keystroke is destroyed before its key can do
/// anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeedShape {
    /// Files, and none of them matching what was typed.
    NoMatch,
    /// The groups and their rows.
    Groups,
}

/// One group's heading and its rows, built once per group title by the keyed
/// `<For>` above.
///
/// The rows come from `arranged` rather than from the value the `<For>` hands
/// in: a keyed child is built once per key, so a captured membership would never
/// narrow.
fn file_group(
    title: Option<String>,
    arranged: Memo<Vec<FileGroup>>,
    addresses: HashMap<String, S3PackageUri>,
    copied: RwSignal<Option<Copied>>,
) -> AnyView {
    let key = title.clone();
    let files = Signal::derive(move || {
        arranged.with(|groups| {
            groups
                .iter()
                .find(|group| group.title == key)
                .map(|group| group.files.clone())
                .unwrap_or_default()
        })
    });
    let heading = title.map(|heading| {
        view! {
            {move || {
                // The count is the length of the rows that follow, always,
                // including one. Never written by hand.
                let count = files.with(Vec::len);
                view! { <GroupHeading title=heading.clone() count=count /> }
            }}
        }
    });
    view! {
        {heading}
        <ul class=rows_class() role="list">
            // Keyed on namespace and path together: a path alone is unique only
            // within its package, and the flat axis draws every package's files
            // in one list.
            <For
                each=move || files.get()
                key=|f: &MainPageFileData| (f.namespace.clone(), f.path.clone())
                children=move |f| file_row(&f, &addresses, copied)
            />
        </ul>
    }
    .into_any()
}

/// One file, wherever it is drawn — a flat row or a row under a `GroupHeading`.
/// Pulled out so the two call sites in [`RecentFilesRegion`] draw the identical
/// row rather than two copies of the same closures drifting apart.
fn file_row(
    f: &MainPageFileData,
    addresses: &HashMap<String, S3PackageUri>,
    copied: RwSignal<Option<Copied>>,
) -> impl IntoView + use<> {
    let namespace = f.namespace.clone();
    // `None` for a package with no bucket: no address, so no button.
    let on_copy_uri: Option<Callback<MouseEvent>> = addresses.get(&f.namespace).map(|package| {
        let package = package.clone();
        let path = f.path.clone();
        Callback::new(move |_| copy_uri(&package, &path, copied))
    });
    let uri = addresses
        .get(&f.namespace)
        .map(|package| crate::util::file_uri(package, &f.path).display());
    let is_copied = Signal::derive(move || {
        uri.as_ref()
            .is_some_and(|uri| copied.with(|c| c.as_ref().is_some_and(|c| &c.uri == uri)))
    });
    let open_ns = namespace.clone();
    let open_path = f.path.clone();
    let reveal_ns = namespace.clone();
    let reveal_path = f.path.clone();
    view! {
        <li>
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
            on_copy_uri=on_copy_uri
            copied=is_copied
        />
        </li>
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
        mount_feed_addressed(files, axis, HashMap::new())
    }

    /// `mount_feed_grouped`, with the package addresses the Copy action needs.
    fn mount_feed_addressed(
        files: Vec<MainPageFileData>,
        axis: &str,
        addresses: HashMap<String, S3PackageUri>,
    ) -> web_sys::Element {
        let axis = axis.to_string();
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <RecentFilesRegion
                        files=files
                        query=Signal::stored(String::new())
                        group_by=Signal::stored(axis)
                        addresses=addresses
                    />
                </leptos_router::components::Router>
            }
        })
    }

    /// The address of a package that has a bucket.
    fn addressed(namespace: &str) -> HashMap<String, S3PackageUri> {
        let uri =
            crate::util::package_uri("team-bucket", namespace, None).expect("a package address");
        HashMap::from([(namespace.to_string(), uri)])
    }

    /// Every action button in the feed, by its accessible name.
    fn action_names(el: &web_sys::Element) -> Vec<String> {
        let buttons = el.query_selector_all("button[aria-label]").unwrap();
        (0..buttons.length())
            .map(|i| {
                let b: web_sys::Element = buttons.item(i).unwrap().dyn_into().unwrap();
                b.get_attribute("aria-label").unwrap_or_default()
            })
            .collect()
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
            href.contains("namespace=user%2Falpha"),
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

    #[wasm_bindgen_test]
    fn a_package_with_an_address_offers_the_copy_action() {
        let el = mount_feed_addressed(
            vec![file("one.csv", "user/alpha", 1_000.0)],
            GROUP_NONE,
            addressed("user/alpha"),
        );

        assert!(
            action_names(&el).iter().any(|n| n == "Copy Quilt+S3 URI"),
            "got: {:?}",
            action_names(&el)
        );
    }

    #[wasm_bindgen_test]
    fn a_package_with_no_bucket_offers_no_copy_action_at_all() {
        // Absent, not inert. A local-only package has no `quilt+s3` address —
        // there is no bucket to name — and a button that cannot do its job is
        // worse than no button.
        let el = mount_feed(vec![file("one.csv", "user/alpha", 1_000.0)]);

        assert!(
            !action_names(&el).iter().any(|n| n == "Copy Quilt+S3 URI"),
            "got: {:?}",
            action_names(&el)
        );
        assert!(
            action_names(&el).iter().any(|n| n == "Reveal in directory"),
            "and the actions that do work are still there"
        );
    }

    /// The card's live region exists before any copy and is empty — a region
    /// created by the event it is meant to announce never fires.
    #[wasm_bindgen_test]
    fn the_card_holds_a_silent_live_region_until_something_is_copied() {
        let el = mount_feed_addressed(
            vec![file("one.csv", "user/alpha", 1_000.0)],
            GROUP_NONE,
            addressed("user/alpha"),
        );

        let region = el
            .query_selector("[role=status]")
            .unwrap()
            .expect("a live region");
        assert!(region.has_attribute("data-sr-only"), "and it is not drawn");
        assert_eq!(region.text_content().unwrap_or_default(), "");
        assert!(
            el.query_selector("li [role=status]").unwrap().is_none(),
            "and it is outside the rows, which are rebuilt by every re-arrangement"
        );
    }
}
