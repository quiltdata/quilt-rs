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

use leptos::prelude::*;
use leptos::task::spawn_local;
use quilt_uri::Namespace;
use quilt_uri::S3PackageUri;

use super::super::main_page::rows_class;
use crate::commands::MainPageFileData;
use crate::kit::Blankslate;
use crate::kit::Card;
use crate::kit::FileRow;
use crate::kit::GroupHeading;
use crate::kit::NameTooltip;

#[cfg(test)]
use super::GROUP_NONE;
use super::GROUP_PACKAGE;
use super::package_page_href;
use crate::pages::copy_mark::CopyMark;
use crate::pages::row_menu::{Ran, RowCommand, carry_out, local_path, menu_actions};

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
    /// its rows offer neither Copy URI nor Open in catalog. One with no
    /// catalog host offers Copy URI alone. The feed's own
    /// payload carries no bucket, which is why this comes from the caller
    /// rather than from `files`.
    #[prop(optional)]
    addresses: HashMap<Namespace, S3PackageUri>,
    /// Whether each package's status check counted no changed files, by
    /// namespace. A package absent here, or not yet checked, counts as changed:
    /// its rows offer neither Copy URI nor Open in catalog.
    #[prop(optional)]
    unchanged: HashMap<Namespace, Signal<bool>>,
) -> impl IntoView {
    // One state rather than a flag per row: exactly one copy can be the most
    // recent, and the rows read it through their own `copied` prop.
    let state = CopyState::new();

    if files.is_empty() {
        return view! {
            <Card label="Recent files" bare=true>
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
        // `BTreeMap` gives the group order, and it is the namespace's own —
        // owner, then package — not the display string's, which sorts `/` after
        // `-` and would put `acme-labs/x` ahead of `acme/y`. Pushing in arrival
        // order keeps the backend's newest-first order inside a group, which
        // nothing here re-derives.
        let mut by_namespace: BTreeMap<Namespace, Vec<MainPageFileData>> = BTreeMap::new();
        for f in visible {
            by_namespace.entry(f.namespace.clone()).or_default().push(f);
        }
        by_namespace
            .into_iter()
            .map(|(namespace, files)| FileGroup {
                title: Some(namespace.to_string()),
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
        // One tooltip for every row's cut path or package tag. Around the card,
        // not inside it: the card spaces its body's children itself.
        <NameTooltip>
        <Card label="Recent files" bare=true>
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
                    let unchanged = unchanged.clone();
                    view! {
                        <For
                            each=move || arranged.get()
                            key=|group: &FileGroup| group.title.clone()
                            children=move |group| {
                                file_group(
                                    group.title,
                                    arranged,
                                    addresses.clone(),
                                    unchanged.clone(),
                                    state,
                                )
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
                {move || state.said.get()}
            </span>
        </Card>
        </NameTooltip>
    }
    .into_any()
}

/// The address that is on the clipboard, and the row it came from.
#[derive(Clone, Debug, PartialEq)]
struct Copied {
    uri: String,
    path: String,
}

type CopyState = CopyMark<Copied>;

/// Runs what a row's `[⋯]` or a click on the row chose. A failure is logged,
/// never rendered: the words a reader sees come only from the kit. A copy
/// reports on the row and the card's live region, and only one the backend
/// confirmed draws a mark; a row that marked itself on click would be wrong in
/// the case that matters, the clipboard refusing.
fn run_command(
    command: RowCommand,
    namespace: Namespace,
    address: Option<S3PackageUri>,
    state: CopyState,
) {
    let copying = matches!(command, RowCommand::CopyUri(_));
    let stamp = if copying { state.bump() } else { 0 };
    spawn_local(async move {
        let ran = carry_out(
            &command,
            &namespace.to_string(),
            address.as_ref(),
            local_path,
            || !copying || state.is_current(stamp),
        )
        .await;
        match ran {
            Ran::Done => {}
            Ran::Copied(uri) => {
                let path = command.path().to_string();
                let words = format!("Copied the address of {path}");
                state.settle(stamp, Some(Copied { uri, path }), words);
            }
            Ran::Failed { lead, detail } => {
                web_sys::console::error_1(&format!("{lead} {detail}").into());
                if copying {
                    state.settle(stamp, None, "Could not copy the address".to_string());
                }
            }
        }
    });
}

/// The address a row's commands run on: the package's, pinned to the revision
/// it holds when that revision is published. The catalog link opens that
/// revision and *Copy URI* copies the file at it.
fn held_address(address: Option<&S3PackageUri>, published: Option<&str>) -> Option<S3PackageUri> {
    address.map(|address| match published {
        Some(hash) => S3PackageUri {
            revision: quilt_uri::RevisionPointer::Hash(hash.to_string()),
            ..address.clone()
        },
        None => address.clone(),
    })
}

/// The items a row's `[⋯]` offers: open and reveal; then the catalog and the
/// copy, which name the published revision the package holds. Both need a
/// bucket, that revision, and a package with no changed files: this page
/// cannot tell which file changed, and the revision would not hold a changed
/// one as it is here. The catalog also needs a catalog host; without one the
/// copy names the bucket alone. No *Ignore*: it edits the package's
/// `.quiltignore` under the package page's lock, which this page does not hold.
fn row_commands(
    path: &str,
    address: Option<&S3PackageUri>,
    published: bool,
    unchanged: bool,
) -> Vec<RowCommand> {
    let path = path.to_string();
    let mut commands = vec![
        RowCommand::Open(path.clone()),
        RowCommand::Reveal(path.clone()),
    ];
    if let Some(address) = address.filter(|a| !a.bucket.is_empty() && published && unchanged) {
        if address.catalog.is_some() {
            commands.push(RowCommand::OpenInCatalog(path.clone()));
        }
        commands.push(RowCommand::CopyUri(path));
    }
    commands
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
    addresses: HashMap<Namespace, S3PackageUri>,
    unchanged: HashMap<Namespace, Signal<bool>>,
    state: CopyState,
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
                children=move |f| {
                    let address = addresses.get(&f.namespace).cloned();
                    let run_with = held_address(address.as_ref(), f.published_hash.as_deref());
                    let unchanged = unchanged
                        .get(&f.namespace)
                        .copied()
                        .unwrap_or(Signal::stored(false));
                    let namespace = f.namespace.clone();
                    let on_command = Callback::new({
                        let run_with = run_with.clone();
                        move |command| {
                            run_command(command, namespace.clone(), run_with.clone(), state);
                        }
                    });
                    file_row(&f, run_with.as_ref(), unchanged, on_command, state.copied)
                }
            />
        </ul>
    }
    .into_any()
}

/// One file, wherever it is drawn: a flat row or a row under a `GroupHeading`.
/// Pulled out so the two call sites in [`RecentFilesRegion`] draw the identical
/// row. `on_command` runs both the row's own open and its `[⋯]`'s items.
fn file_row(
    f: &MainPageFileData,
    address: Option<&S3PackageUri>,
    unchanged: Signal<bool>,
    on_command: Callback<RowCommand>,
    copied: RwSignal<Option<Copied>>,
) -> impl IntoView + use<> {
    let (path, held, published) = (f.path.clone(), address.cloned(), f.published_hash.is_some());
    let actions = Signal::derive(move || {
        let commands = row_commands(&path, held.as_ref(), published, unchanged.get());
        menu_actions(&commands, on_command)
    });
    let uri = address.map(|package| crate::util::file_uri(package, &f.path).to_copy_string());
    let is_copied = Signal::derive(move || {
        uri.as_ref()
            .is_some_and(|uri| copied.with(|c| c.as_ref().is_some_and(|c| &c.uri == uri)))
    });
    let open = RowCommand::Open(f.path.clone());
    view! {
        <li>
        <FileRow
            path=f.path.clone()
            package=f.namespace.to_string()
            package_href=package_page_href(&f.namespace)
            at=f.changed_at
            on_open=move |_| on_command.run(open.clone())
            actions=actions
            copied=is_copied
        />
        </li>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use crate::test_support::sleep_ms;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The scenes name packages as text; the payloads carry the type.
    fn ns(text: &str) -> Namespace {
        Namespace::try_from(text).expect("a namespace")
    }

    fn file(path: &str, namespace: &str, changed_at: f64) -> MainPageFileData {
        MainPageFileData {
            path: path.to_string(),
            namespace: ns(namespace),
            changed_at,
            published_hash: Some("abc123".to_string()),
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
        mount_feed_addressed(files, axis, HashMap::new(), HashMap::new())
    }

    /// `mount_feed_grouped`, with the package addresses and the change counts
    /// the catalog and Copy actions need.
    fn mount_feed_addressed(
        files: Vec<MainPageFileData>,
        axis: &str,
        addresses: HashMap<Namespace, S3PackageUri>,
        unchanged: HashMap<Namespace, Signal<bool>>,
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
                        unchanged=unchanged
                    />
                </leptos_router::components::Router>
            }
        })
    }

    /// The address of a package that has a bucket and a catalog host.
    fn addressed(namespace: &str) -> HashMap<Namespace, S3PackageUri> {
        let namespace = ns(namespace);
        let uri = crate::util::package_uri("team-bucket", &namespace, Some("quilt.test"));
        HashMap::from([(namespace, uri)])
    }

    /// A package whose status check counted no changed files, as `clean` says.
    fn counted(namespace: &str, clean: Signal<bool>) -> HashMap<Namespace, Signal<bool>> {
        HashMap::from([(ns(namespace), clean)])
    }

    const TRIGGER: &str = "button[aria-label='More actions for this file']";

    /// Open the first row's `[⋯]` and answer its items.
    async fn open_menu(el: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
        el.query_selector(TRIGGER)
            .unwrap()
            .expect("a row's [⋯]")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        leptos::task::tick().await;
        let found = el.query_selector_all("[popover] button").unwrap();
        (0..found.length())
            .map(|i| found.item(i).unwrap().unchecked_into())
            .collect()
    }

    fn item(items: &[web_sys::HtmlElement], words: &str) -> web_sys::HtmlElement {
        items
            .iter()
            .find(|b| b.text_content().unwrap_or_default() == words)
            .unwrap_or_else(|| panic!("no item {words}"))
            .clone()
    }

    /// A row's items, each marked `|` where a rule goes above it.
    fn menu(
        path: &str,
        address: Option<&S3PackageUri>,
        published: bool,
        unchanged: bool,
    ) -> Vec<String> {
        let commands = row_commands(path, address, published, unchanged);
        commands
            .iter()
            .zip(crate::pages::row_menu::group_starts(&commands))
            .map(|(command, starts)| {
                let rule = if starts { "| " } else { "" };
                format!("{rule}{}", command.label())
            })
            .collect()
    }

    fn with_catalog() -> S3PackageUri {
        crate::util::package_uri("team-bucket", &ns("user/alpha"), Some("quilt.test"))
    }

    #[test]
    fn a_published_package_with_a_catalog_offers_every_item() {
        assert_eq!(
            menu("one.csv", Some(&with_catalog()), true, true),
            [
                "Open file",
                "Reveal in folder",
                "| Open in catalog",
                "Copy URI"
            ]
        );
    }

    /// An unpublished revision may hold files the catalog lacks, so none of the
    /// package's rows link there or copy an address.
    #[test]
    fn a_package_ahead_of_its_remote_offers_only_the_local_file() {
        assert_eq!(
            menu("one.csv", Some(&with_catalog()), false, true),
            ["Open file", "Reveal in folder"]
        );
    }

    /// The held revision names a package's files by path, and this page cannot
    /// tell which file changed: any change hides both items on every row.
    #[test]
    fn a_package_with_local_changes_offers_only_the_local_file() {
        assert_eq!(
            menu("one.csv", Some(&with_catalog()), true, false),
            ["Open file", "Reveal in folder"]
        );
    }

    #[test]
    fn a_bucket_with_no_catalog_host_offers_the_address_alone() {
        let bucket_only = crate::util::package_uri("team-bucket", &ns("user/alpha"), None);
        assert_eq!(
            menu("one.csv", Some(&bucket_only), true, true),
            ["Open file", "Reveal in folder", "| Copy URI"]
        );
        assert_eq!(
            menu("one.csv", Some(&bucket_only), false, true),
            ["Open file", "Reveal in folder"],
            "an unpublished revision"
        );
        assert_eq!(
            menu("one.csv", Some(&bucket_only), true, false),
            ["Open file", "Reveal in folder"],
            "local changes"
        );
    }

    /// Without a catalog host, the copied address names the bucket and the
    /// held revision, and no catalog.
    #[wasm_bindgen_test]
    async fn without_a_catalog_the_copy_names_the_bucket() {
        let bucket_only = crate::util::package_uri("team-bucket", &ns("user/alpha"), None);
        let held = held_address(Some(&bucket_only), Some("abc123")).expect("an address");
        let (copied, _) = crate::pages::row_menu::clipped(
            &RowCommand::CopyUri("one.csv".to_string()),
            "user/alpha",
            Some(&held),
            local_path,
        )
        .await
        .unwrap()
        .expect("an address");
        assert_eq!(
            copied,
            "quilt+s3://team-bucket#package=user/alpha@abc123&path=one.csv"
        );
    }

    /// A local-only package has no `quilt+s3` address: no bucket to name.
    #[test]
    fn a_package_with_no_bucket_offers_only_the_local_file() {
        assert_eq!(
            menu("one.csv", None, false, true),
            ["Open file", "Reveal in folder"]
        );
        assert_eq!(
            menu("one.csv", None, true, true),
            ["Open file", "Reveal in folder"]
        );
    }

    /// A package behind its remote names the revision it holds, as the package
    /// page does, not the remote's latest: the catalog link, and the copied
    /// address with its whole hash.
    #[wasm_bindgen_test]
    async fn the_catalog_link_and_the_copy_name_the_held_revision() {
        let hash = "0123456789abcdef".repeat(4);
        let held = held_address(Some(&with_catalog()), Some(&hash)).expect("an address");
        let link = crate::util::entry_catalog_url(&held, "one.csv").expect("a catalog link");
        let latest = crate::util::entry_catalog_url(&with_catalog(), "one.csv").unwrap();
        assert!(link.contains(&hash), "got {link}");
        assert_ne!(link, latest);

        let (copied, _) = crate::pages::row_menu::clipped(
            &RowCommand::CopyUri("one.csv".to_string()),
            "user/alpha",
            Some(&held),
            local_path,
        )
        .await
        .unwrap()
        .expect("an address");
        assert_eq!(
            copied,
            format!(
                "quilt+s3://team-bucket#package=user/alpha@{hash}&path=one.csv&catalog=quilt.test"
            )
        );
        assert_eq!(
            held_address(Some(&with_catalog()), None),
            Some(with_catalog())
        );
    }

    /// An item hands its command on, and so does a click on the row; neither
    /// does the other's.
    #[wasm_bindgen_test]
    async fn choosing_an_item_hands_its_command_on() {
        let chosen = RwSignal::new(Vec::<RowCommand>::new());
        let el = mount(move || {
            let on_command = Callback::new(move |c| chosen.update(|all| all.push(c)));
            let f = file("raw/one.csv", "user/alpha", 1_000.0);
            view! {
                <leptos_router::components::Router>
                    {file_row(&f, Some(&with_catalog()), Signal::stored(true), on_command, RwSignal::new(None))}
                </leptos_router::components::Router>
            }
        });

        item(&open_menu(&el).await, "Open in catalog").click();
        assert_eq!(
            chosen.get_untracked(),
            [RowCommand::OpenInCatalog("raw/one.csv".to_string())]
        );

        el.query_selector("li [class*=path]")
            .unwrap()
            .expect("the path")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        assert_eq!(
            chosen.get_untracked().last(),
            Some(&RowCommand::Open("raw/one.csv".to_string()))
        );
        assert_eq!(chosen.get_untracked().len(), 2);
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

    /// The feed holds one tooltip for every row's path and tag, flat or
    /// grouped, and no path or tag carries a `title`.
    #[wasm_bindgen_test]
    fn the_feed_mounts_one_tooltip_for_all_its_rows() {
        let files = || {
            vec![
                file("b/two.csv", "user/beta", 5_000.0),
                file("c/three.csv", "user/gamma", 9_000.0),
                file("a/one.csv", "user/alpha", 1_000.0),
            ]
        };
        for el in [
            mount_feed(files()),
            mount_feed_grouped(files(), GROUP_PACKAGE),
        ] {
            assert_eq!(
                el.query_selector_all("[role='tooltip']").unwrap().length(),
                1,
                "markup was {}",
                el.inner_html()
            );
            assert_eq!(
                el.query_selector_all("[data-full-name]").unwrap().length(),
                6
            );
            assert!(
                el.query_selector("[data-full-name][title]")
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[expect(dead_code, reason = "only the classes this layout needs are loaded")]
    mod sheets {
        stylance::import_crate_style!(pub page, "src/pages/main_page.module.scss");
        stylance::import_crate_style!(pub card, "src/kit/card.module.scss");
    }

    /// The feed has no scroller of its own: a long feed grows the region, and
    /// the page scrolls it, so the wheel moves the list from anywhere on the
    /// window.
    #[wasm_bindgen_test]
    async fn a_long_feed_scrolls_the_page_not_its_card() {
        use crate::test_support::load_module_style;
        load_module_style(
            include_str!("../main_page.module.scss"),
            &[("list_region", sheets::page::list_region)],
        );
        load_module_style(
            include_str!("../../kit/card.module.scss"),
            &[
                ("root", sheets::card::root),
                ("body", sheets::card::body),
                ("bare", sheets::card::bare),
            ],
        );
        let files: Vec<_> = (0..60)
            .map(|i| file(&format!("data/{i}.csv"), "user/alpha", f64::from(i)))
            .collect();
        let el = mount(move || {
            view! {
                <leptos_router::components::Router>
                    <div data-page style="height:300px; overflow-y:auto">
                        <div class=sheets::page::list_region>
                            <div style="height:32px">"Toolbar"</div>
                            <RecentFilesRegion
                                files=files.clone()
                                query=Signal::stored(String::new())
                                group_by=Signal::stored(GROUP_NONE.to_string())
                            />
                        </div>
                    </div>
                </leptos_router::components::Router>
            }
        });
        sleep_ms(0).await;
        let card: web_sys::HtmlElement = el
            .query_selector("section")
            .unwrap()
            .expect("the card")
            .unchecked_into();
        assert_eq!(
            card.scroll_height(),
            card.client_height(),
            "the card scrolls nothing itself"
        );
        let page: web_sys::HtmlElement = el
            .query_selector("[data-page]")
            .unwrap()
            .expect("the page")
            .unchecked_into();
        assert!(
            page.scroll_height() > page.client_height(),
            "the page scrolls the rows"
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

    /// The one case where the namespace's order and its text's disagree: an
    /// owner that is a prefix of another owner. `acme` sorts before `acme-labs`
    /// as owners do, though `acme-labs/plate` would come first as text.
    #[wasm_bindgen_test]
    fn the_feed_orders_owners_as_owners_not_as_text() {
        let el = mount_feed_grouped(
            vec![
                file("one.csv", "acme-labs/plate", 9_000.0),
                file("two.csv", "acme/plate", 1_000.0),
            ],
            GROUP_PACKAGE,
        );

        let text = el.text_content().unwrap();
        let acme = text.find("acme/plate").expect("acme");
        let labs = text.find("acme-labs/plate").expect("acme-labs");
        assert!(acme < labs, "got: {text}");
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

    /// The items follow the package's count: a row offers them once its
    /// package is checked and clean, and drops them when a change is counted.
    #[wasm_bindgen_test]
    async fn a_counted_change_takes_the_address_items_away() {
        let clean = RwSignal::new(false);
        let el = mount_feed_addressed(
            vec![file("one.csv", "user/alpha", 1_000.0)],
            GROUP_NONE,
            addressed("user/alpha"),
            counted("user/alpha", clean.into()),
        );
        let words = |items: Vec<web_sys::HtmlElement>| -> Vec<String> {
            items
                .iter()
                .map(|b| b.text_content().unwrap_or_default())
                .collect()
        };
        assert_eq!(
            words(open_menu(&el).await),
            ["Open file", "Reveal in folder"]
        );
        clean.set(true);
        leptos::task::tick().await;
        let found = el.query_selector_all("[popover] button").unwrap();
        let open: Vec<web_sys::HtmlElement> = (0..found.length())
            .map(|i| found.item(i).unwrap().unchecked_into())
            .collect();
        assert_eq!(
            words(open),
            [
                "Open file",
                "Reveal in folder",
                "Open in catalog",
                "Copy URI"
            ]
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
            counted("user/alpha", Signal::stored(true)),
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

    /// A refused clipboard says so. Silence would be indistinguishable from a
    /// click that never registered, and there is no Tauri bridge in this
    /// harness, so every copy here refuses.
    #[wasm_bindgen_test]
    async fn a_copy_that_fails_says_so_and_draws_no_mark() {
        let el = mount_feed_addressed(
            vec![file("one.csv", "user/alpha", 1_000.0)],
            GROUP_NONE,
            addressed("user/alpha"),
            counted("user/alpha", Signal::stored(true)),
        );

        item(&open_menu(&el).await, "Copy URI").click();
        sleep_ms(50).await;

        let region = el
            .query_selector("[role=status]")
            .unwrap()
            .expect("a live region");
        assert_eq!(
            region.text_content().unwrap_or_default(),
            "Could not copy the address"
        );
        assert!(
            !el.text_content().unwrap_or_default().contains("Copied"),
            "and the row does not claim a copy that did not happen"
        );
    }
}
