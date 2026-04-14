use leptos::prelude::*;

#[component]
pub fn IconLink(
    href: String,
    icon: &'static str,
    #[prop(optional)]
    small: bool,
    #[prop(optional)]
    primary: bool,
    children: Children,
) -> impl IntoView {
    let class = match (small, primary) {
        (false, false) => "qui-button",
        (true, false) => "qui-button small",
        (false, true) => "qui-button primary",
        (true, true) => "qui-button primary small",
    };

    view! {
        <a class=class href=href>
            <img class="qui-icon" src=icon />
            <span>{children()}</span>
        </a>
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
    fn renders_anchor_tag() {
        let el = mount(|| view! {
            <IconLink href="/test".to_string() icon="/icons/test.svg">"Label"</IconLink>
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(link.tag_name(), "A");
    }

    #[wasm_bindgen_test]
    fn renders_href() {
        let el = mount(|| view! {
            <IconLink href="/some/path".to_string() icon="/icons/test.svg">"X"</IconLink>
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(link.get_attribute("href").unwrap(), "/some/path");
    }

    #[wasm_bindgen_test]
    fn renders_icon() {
        let el = mount(|| view! {
            <IconLink href="/x".to_string() icon="/icons/custom.svg">"X"</IconLink>
        });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(icon.get_attribute("src").unwrap(), "/icons/custom.svg");
    }

    #[wasm_bindgen_test]
    fn renders_label_in_span() {
        let el = mount(|| view! {
            <IconLink href="/x".to_string() icon="/icons/x.svg">"My Label"</IconLink>
        });
        let span = el.query_selector("a > span").unwrap().unwrap();
        assert_eq!(span.text_content().unwrap(), "My Label");
    }

    #[wasm_bindgen_test]
    fn default_classes() {
        let el = mount(|| view! {
            <IconLink href="/x".to_string() icon="/icons/x.svg">"X"</IconLink>
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(!link.class_list().contains("small"));
        assert!(!link.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn small_class() {
        let el = mount(|| view! {
            <IconLink href="/x".to_string() icon="/icons/x.svg" small=true>"X"</IconLink>
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(link.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn primary_class() {
        let el = mount(|| view! {
            <IconLink href="/x".to_string() icon="/icons/x.svg" primary=true>"X"</IconLink>
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(link.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn small_and_primary_classes() {
        let el = mount(|| view! {
            <IconLink href="/x".to_string() icon="/icons/x.svg" small=true primary=true>"X"</IconLink>
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(link.class_list().contains("small"));
        assert!(link.class_list().contains("primary"));
    }
}
