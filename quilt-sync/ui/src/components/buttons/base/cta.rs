use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

/// Call-to-action button — the primary action the page exists for.
/// Always large and visually prominent; icon renders after the label
/// (trailing position). Use `IconButton` for secondary actions.
#[component]
pub fn ButtonCta(
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(optional)] on_click: Option<UnsyncCallback<leptos::ev::MouseEvent>>,
    #[prop(optional, into)] primary: MaybeProp<bool>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <button
            class="qui-button large"
            class:primary=move || primary.get().unwrap_or(false)
            type="button"
            prop:disabled=move || disabled.get().unwrap_or(false)
            on:click=move |ev| {
                if let Some(cb) = &on_click {
                    cb.run(ev);
                }
            }
        >
            <span>{children()}</span>
            {icon.map(|src| view! { <img class="qui-icon" src=src /> })}
        </button>
    }
}

#[component]
pub fn CtaLink(
    href: String,
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(optional, into)] primary: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <a
            class="qui-button large"
            class:primary=move || primary.get().unwrap_or(false)
            href=href
        >
            <span>{children()}</span>
            {icon.map(|src| view! { <img class="qui-icon" src=src /> })}
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

    // ── ButtonCta ──

    #[wasm_bindgen_test]
    fn renders_button_with_large_class() {
        let el = mount(|| {
            view! {
                <ButtonCta on_click=UnsyncCallback::new(|_| {})>"Go"</ButtonCta>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("large"));
    }

    #[wasm_bindgen_test]
    fn primary_class() {
        let el = mount(|| {
            view! {
                <ButtonCta on_click=UnsyncCallback::new(|_| {}) primary=true>"Go"</ButtonCta>
            }
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn trailing_icon() {
        let el = mount(|| {
            view! {
                <ButtonCta on_click=UnsyncCallback::new(|_| {}) icon="/icons/arrow.svg">"Go"</ButtonCta>
            }
        });
        // Icon should come after span
        let btn = el.query_selector("button").unwrap().unwrap();
        let children: Vec<web_sys::Element> = (0..btn.child_element_count())
            .filter_map(|i| btn.children().item(i))
            .collect();
        assert_eq!(children[0].tag_name(), "SPAN");
        assert_eq!(children[1].tag_name(), "IMG");
    }

    #[wasm_bindgen_test]
    fn no_icon_when_omitted() {
        let el = mount(|| {
            view! {
                <ButtonCta on_click=UnsyncCallback::new(|_| {})>"Go"</ButtonCta>
            }
        });
        assert!(el.query_selector("img").unwrap().is_none());
    }

    #[wasm_bindgen_test]
    fn disabled_when_set() {
        let el = mount(|| {
            view! {
                <ButtonCta on_click=UnsyncCallback::new(|_| {}) disabled=true>"Go"</ButtonCta>
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
                <ButtonCta on_click=UnsyncCallback::new(move |_| clicked_clone.set(true))>"Go"</ButtonCta>
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

    // ── CtaLink ──

    #[wasm_bindgen_test]
    fn renders_anchor_when_href_provided() {
        let el = mount(|| {
            view! {
                <CtaLink href="/test".to_string()>"Go"</CtaLink>
            }
        });
        let link = el.query_selector("a").unwrap().unwrap();
        assert_eq!(link.tag_name(), "A");
        assert!(link.class_list().contains("large"));
        assert!(el.query_selector("button").unwrap().is_none());
    }
}
