use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

#[component]
pub fn IconButton(
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(optional)] on_click: Option<UnsyncCallback<leptos::ev::MouseEvent>>,
    #[prop(optional)] small: bool,
    #[prop(optional)] primary: bool,
    #[prop(optional)] warning: bool,
    #[prop(optional)] large: bool,
    #[prop(optional)] link: bool,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <button
            class="qui-button"
            class:primary=primary
            class:warning=warning
            class:small=small
            class:large=large
            class:link=link
            type="button"
            prop:disabled=move || disabled.get().unwrap_or(false)
            on:click=move |ev| {
                if let Some(cb) = &on_click {
                    cb.run(ev);
                }
            }
        >
            {icon.map(|src| view! { <img class="qui-icon" src=src /> })}
            <span>{children()}</span>
        </button>
    }
}

#[component]
pub fn IconLink(
    href: String,
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(optional)] small: bool,
    #[prop(optional)] primary: bool,
    #[prop(optional)] warning: bool,
    #[prop(optional)] large: bool,
    #[prop(optional)] link: bool,
    children: Children,
) -> impl IntoView {
    view! {
        <a
            class="qui-button"
            class:primary=primary
            class:warning=warning
            class:small=small
            class:large=large
            class:link=link
            href=href
        >
            {icon.map(|src| view! { <img class="qui-icon" src=src /> })}
            <span>{children()}</span>
        </a>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
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

    // ── IconButton ──

    #[wasm_bindgen_test]
    fn renders_button_tag() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/test.svg" on_click=UnsyncCallback::new(|_| {})>"Label"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert_eq!(btn.tag_name(), "BUTTON");
        assert_eq!(btn.get_attribute("type").unwrap(), "button");
    }

    #[wasm_bindgen_test]
    fn renders_icon() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/custom.svg" on_click=UnsyncCallback::new(|_| {})>"X"</IconButton>
            }
        });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(icon.get_attribute("src").unwrap(), "/icons/custom.svg");
    }

    #[wasm_bindgen_test]
    fn renders_label_in_span() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {})>"My Label"</IconButton>
            }
        });
        let span = el.query_selector("button > span").unwrap().unwrap();
        assert_eq!(span.text_content().unwrap(), "My Label");
    }

    #[wasm_bindgen_test]
    fn default_classes() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {})>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(!btn.class_list().contains("small"));
        assert!(!btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn small_class() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) small=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn primary_class() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) primary=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn small_and_primary_classes() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) small=true primary=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("small"));
        assert!(btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn warning_class() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) warning=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("warning"));
    }

    #[wasm_bindgen_test]
    fn warning_and_small_classes() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) warning=true small=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("warning"));
        assert!(btn.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn large_class() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) large=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("large"));
    }

    #[wasm_bindgen_test]
    fn link_style_class() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) link=true>"X"</IconButton>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("link"));
    }

    #[wasm_bindgen_test]
    fn not_disabled_by_default() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {})>"X"</IconButton>
            }
        });
        let btn: web_sys::HtmlButtonElement = el
            .query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!btn.disabled());
    }

    #[wasm_bindgen_test]
    fn disabled_when_set() {
        let el = mount(|| {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(|_| {}) disabled=true>"X"</IconButton>
            }
        });
        let btn: web_sys::HtmlButtonElement = el
            .query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(btn.disabled());
    }

    #[wasm_bindgen_test]
    fn click_fires_handler() {
        let clicked = Rc::new(Cell::new(false));
        let clicked_clone = clicked.clone();
        let el = mount(move || {
            view! {
                <IconButton icon="/icons/x.svg" on_click=UnsyncCallback::new(move |_| clicked_clone.set(true))>"X"</IconButton>
            }
        });
        let btn: web_sys::HtmlElement = el
            .query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        btn.click();
        assert!(clicked.get());
    }

    #[wasm_bindgen_test]
    fn no_icon_when_omitted() {
        let el = mount(|| {
            view! {
                <IconButton on_click=UnsyncCallback::new(|_| {})>"Label"</IconButton>
            }
        });
        assert!(el.query_selector("img.qui-icon").unwrap().is_none());
        let span = el.query_selector("button > span").unwrap().unwrap();
        assert_eq!(span.text_content().unwrap(), "Label");
    }

    // ── IconLink ──

    #[wasm_bindgen_test]
    fn renders_anchor_when_href_provided() {
        let el = mount(|| {
            view! {
                <IconLink icon="/icons/test.svg" href="/test".to_string()>"Label"</IconLink>
            }
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(link.tag_name(), "A");
        assert!(el.query_selector("button").unwrap().is_none());
    }

    #[wasm_bindgen_test]
    fn renders_href() {
        let el = mount(|| {
            view! {
                <IconLink icon="/icons/test.svg" href="/some/path".to_string()>"X"</IconLink>
            }
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(link.get_attribute("href").unwrap(), "/some/path");
    }

    #[wasm_bindgen_test]
    fn renders_label_in_span_for_link() {
        let el = mount(|| {
            view! {
                <IconLink icon="/icons/x.svg" href="/x".to_string()>"My Label"</IconLink>
            }
        });
        let span = el.query_selector("a > span").unwrap().unwrap();
        assert_eq!(span.text_content().unwrap(), "My Label");
    }

    #[wasm_bindgen_test]
    fn link_classes() {
        let el = mount(|| {
            view! {
                <IconLink icon="/icons/x.svg" href="/x".to_string() small=true primary=true>"X"</IconLink>
            }
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert!(link.class_list().contains("qui-button"));
        assert!(link.class_list().contains("small"));
        assert!(link.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn link_renders_icon() {
        let el = mount(|| {
            view! {
                <IconLink icon="/icons/custom.svg" href="/x".to_string()>"X"</IconLink>
            }
        });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(icon.get_attribute("src").unwrap(), "/icons/custom.svg");
    }
}
