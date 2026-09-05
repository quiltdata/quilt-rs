//! §3.2's feed: one row per file, flat, in the order the backend sent it. The
//! ordering and the cap are the backend's (§4.5) and are not re-derived here —
//! two sources of truth for "newest" is how they drift.
//!
//! A file row carries no state, no tone and no `provisional`: a path and an
//! mtime are facts on disk. That is why this view was buildable before the
//! status scales were settled, and why nothing here imports [`crate::kit::render`].

use leptos::prelude::*;

use crate::commands;
use crate::commands::MainPageFileData;
use crate::kit::Blankslate;
use crate::kit::Card;
use crate::kit::FileRow;

use super::package_page_href;

/// The card, on one payload — no title, because the toggle above it (a later
/// plan) names the view; see `Card::title`'s own doc for why.
///
/// Mounted by a later task (Plan 7, Task 4); until then only this file's own
/// tests construct it (see `mod recent_files`'s suppression in `main_page.rs`).
#[component]
pub fn RecentFilesRegion(files: Vec<MainPageFileData>) -> impl IntoView {
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
            {files
                .into_iter()
                .map(|f| {
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
                                    if let Err(err) =
                                        commands::open_in_default_application(ns, path, None).await
                                    {
                                        // Logged, never rendered: the words a user
                                        // reads come only from the kit.
                                        web_sys::console::error_1(
                                            &format!("open_in_default_application failed: {err}")
                                                .into(),
                                        );
                                    }
                                });
                            }
                            on_reveal=move |_| {
                                let (ns, path) = (reveal_ns.clone(), reveal_path.clone());
                                leptos::task::spawn_local(async move {
                                    if let Err(err) =
                                        commands::reveal_in_file_browser(ns, path, None).await
                                    {
                                        web_sys::console::error_1(
                                            &format!("reveal_in_file_browser failed: {err}")
                                                .into(),
                                        );
                                    }
                                });
                            }
                        />
                    }
                })
                .collect_view()}
        </Card>
    }
    .into_any()
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
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <RecentFilesRegion files=files />
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
}
