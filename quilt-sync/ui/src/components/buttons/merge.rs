use leptos::prelude::*;

use super::{ButtonKind, IconLink};

const KIND: ButtonKind = ButtonKind::Merge;

#[component]
pub fn Merge(namespace: String, #[prop(optional)] small: bool) -> impl IntoView {
    let href = format!("/merge?namespace={}", namespace);

    view! {
        <IconLink href=href icon=KIND.icon() small=small primary=true>
            {KIND.label()}
        </IconLink>
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
    fn builds_href_from_namespace() {
        let el = mount(|| view! { <Merge namespace="user/pkg".to_string() /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(
            link.get_attribute("href").unwrap(),
            "/merge?namespace=user/pkg"
        );
    }

    #[wasm_bindgen_test]
    fn uses_merge_icon_and_label() {
        let el = mount(|| view! { <Merge namespace="a/b".to_string() /> });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(
            icon.get_attribute("src").unwrap(),
            "/assets/img/icons/merge.svg"
        );
        assert_eq!(el.text_content().unwrap().trim(), "Merge");
    }

    #[wasm_bindgen_test]
    fn is_primary() {
        let el = mount(|| view! { <Merge namespace="a/b".to_string() /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("primary"));
    }
}
