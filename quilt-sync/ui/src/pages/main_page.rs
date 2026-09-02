//! The v2 main page. Behind `ExperimentalSettings.main_page_v2`.
//!
//! One region so far. `Transition`, never `Suspense` (§6): a refetch fires on
//! every autosync transition, publish and pause, and a `Suspense` boundary
//! re-shows its fallback each time, so the page would strobe.

use leptos::prelude::*;

use crate::commands;
use crate::kit::Card;
use crate::kit::PackageRow;
use crate::kit::PackageRowSkeleton;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::kit::Site;
use crate::kit::render;

/// The rows. Split out from `MainPage` so it can be tested without a Tauri host.
#[component]
fn PackageList(packages: Vec<(String, PackageState, bool)>) -> impl IntoView {
    view! {
        <div class="g-rows">
            {packages
                .into_iter()
                .map(|(namespace, state, provisional)| {
                    let rendered = render(&state, Site::ListRow);
                    view! {
                        <PackageRow
                            namespace=namespace
                            href="/installed-package"
                            state=rendered.words
                            tone=rendered.tone
                            provisional=provisional
                        />
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
pub fn MainPage() -> impl IntoView {
    let packages = LocalResource::new(|| async move { commands::get_main_page_packages().await });

    view! {
        <PageLayout>
            <Card title="Packages">
                <Transition fallback=|| {
                    view! {
                        <div class="g-rows">
                            <PackageRowSkeleton />
                            <PackageRowSkeleton />
                            <PackageRowSkeleton />
                        </div>
                    }
                }>
                    {move || Suspend::new(async move {
                        match packages.await {
                            Ok(data) => {
                                let rows = data
                                    .packages
                                    .into_iter()
                                    .map(|p| (p.namespace, p.state, p.provisional))
                                    .collect();
                                view! { <PackageList packages=rows /> }.into_any()
                            }
                            Err(err) => view! { <p>{err}</p> }.into_any(),
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
        let el = mount(|| view! { <MainPage /> });
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
    fn a_provisional_row_is_marked_provisional() {
        let el = mount(|| {
            view! {
                <PackageList packages=vec![
                    ("user/a".to_string(), PackageState::Latest, true),
                ] />
            }
        });
        assert!(
            el.query_selector("[class*=provisional]").unwrap().is_some(),
            "the light phase's guess is drawn dashed until the heavy phase confirms it"
        );
    }
}
