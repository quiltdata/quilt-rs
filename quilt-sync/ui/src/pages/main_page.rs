//! The v2 main page. Behind `ExperimentalSettings.main_page_v2`.

use leptos::prelude::*;

use crate::kit::Card;
use crate::kit::PageLayout;

#[component]
pub fn MainPage() -> impl IntoView {
    view! {
        <PageLayout>
            <Card title="Packages">
                <div></div>
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
}
