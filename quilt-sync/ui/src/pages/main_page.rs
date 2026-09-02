//! The v2 main page. Behind `ExperimentalSettings.main_page_v2`.
//!
//! One region so far. `Transition`, never `Suspense` (§6): a refetch fires on
//! every autosync transition, publish and pause, and a `Suspense` boundary
//! re-shows its fallback each time, so the page would strobe.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;

/// One row's data: namespace, state, whether the state is still the light phase's
/// guess, and when the copy last changed. A tuple rather than a struct because it is
/// local to this file and never crosses a boundary.
type PackageRowData = (String, PackageState, bool, Option<f64>);

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
        .map(|(namespace, state, provisional, changed_at)| {
            let rendered = render(&state, Site::ListRow);
            // The namespace has to travel in the query string: the package page reads it
            // with `use_query_map`, and a bare path leaves it empty — which is the
            // "Invalid namespace" that page then reports. `filter` matches what v1's list
            // link sends, so the destination behaves the same however you arrived at it.
            let href = format!("/installed-package?namespace={namespace}&filter=unmodified");
            view! {
                <PackageRow
                    namespace=namespace
                    href=href
                    changed_at=changed_at
                    state=rendered.words
                    tone=rendered.tone
                    provisional=provisional
                />
            }
        })
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
                                    .map(|p| (p.namespace, p.state, p.provisional, p.changed_at))
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
                    ("user/plate-07".to_string(), PackageState::Latest, true, None),
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
                    ("user/a".to_string(), PackageState::Latest, true, None),
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
}
