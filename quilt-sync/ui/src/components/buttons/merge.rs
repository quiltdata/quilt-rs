use leptos::prelude::*;

use super::{ButtonKind, IconLink};

use quilt_uri::Namespace;

const KIND: ButtonKind = ButtonKind::Merge;

#[component]
#[allow(clippy::needless_pass_by_value)]
pub fn Merge(namespace: Namespace, #[prop(optional)] small: bool) -> impl IntoView {
    let href = crate::routes::merge_href(&namespace);

    view! {
        <IconLink href=href icon=KIND.icon() small=small primary=true>
            {KIND.label()}
        </IconLink>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// The scenes name packages as text; the payloads carry the type.
    fn ns(text: &str) -> Namespace {
        Namespace::try_from(text).expect("a namespace")
    }

    #[wasm_bindgen_test]
    fn builds_href_from_namespace() {
        let el = mount(|| view! { <Merge namespace=ns("user/pkg") /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(
            link.get_attribute("href").unwrap(),
            "/merge?namespace=user%2Fpkg"
        );
    }

    #[wasm_bindgen_test]
    fn uses_merge_icon_and_label() {
        let el = mount(|| view! { <Merge namespace=ns("a/b") /> });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(
            icon.get_attribute("src").unwrap(),
            "/assets/img/icons/merge.svg"
        );
        assert_eq!(el.text_content().unwrap().trim(), "Resolve conflict");
    }

    #[wasm_bindgen_test]
    fn is_primary() {
        let el = mount(|| view! { <Merge namespace=ns("a/b") /> });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("primary"));
    }
}
