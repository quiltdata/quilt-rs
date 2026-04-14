use leptos::prelude::*;

use super::IconLink;

#[component]
pub fn CommitLink(
    namespace: String,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    let href = format!("/commit?namespace={}", namespace);

    view! {
        <IconLink href=href icon="/assets/img/icons/commit.svg" small=small>
            "Commit"
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
    fn renders_link_with_correct_href() {
        let el = mount(|| view! { <CommitLink namespace="user/pkg".to_string() /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(link.get_attribute("href").unwrap(), "/commit?namespace=user/pkg");
    }

    #[wasm_bindgen_test]
    fn renders_default_classes() {
        let el = mount(|| view! { <CommitLink namespace="a/b".to_string() /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(!link.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn renders_small_class() {
        let el = mount(|| view! { <CommitLink namespace="a/b".to_string() small=true /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(link.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn renders_icon_and_label() {
        let el = mount(|| view! { <CommitLink namespace="a/b".to_string() /> });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(icon.get_attribute("src").unwrap(), "/assets/img/icons/commit.svg");
        assert_eq!(el.text_content().unwrap().trim(), "Commit");
    }
}
