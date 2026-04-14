use leptos::prelude::*;

#[component]
pub fn IconButton(
    icon: &'static str,
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
    #[prop(optional)]
    primary: bool,
    #[prop(optional)]
    warning: bool,
    #[prop(optional, into)]
    disabled: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    let class = match (small, primary, warning) {
        (false, false, false) => "qui-button",
        (true, false, false) => "qui-button small",
        (false, true, false) => "qui-button primary",
        (true, true, false) => "qui-button primary small",
        (false, false, true) => "qui-button warning",
        (true, false, true) => "qui-button warning small",
        (false, true, true) => "qui-button primary warning",
        (true, true, true) => "qui-button primary warning small",
    };

    view! {
        <button
            class=class
            type="button"
            prop:disabled=move || disabled.get().unwrap_or(false)
            on:click=on_click
        >
            <img class="qui-icon" src=icon />
            <span>{children()}</span>
        </button>
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

    #[wasm_bindgen_test]
    fn renders_button_tag() {
        let el = mount(|| view! {
            <IconButton icon="/icons/test.svg" on_click=|_| {}>"Label"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert_eq!(btn.tag_name(), "BUTTON");
        assert_eq!(btn.get_attribute("type").unwrap(), "button");
    }

    #[wasm_bindgen_test]
    fn renders_icon() {
        let el = mount(|| view! {
            <IconButton icon="/icons/custom.svg" on_click=|_| {}>"X"</IconButton>
        });
        let icon = el.query_selector("img.qui-icon").unwrap().unwrap();
        assert_eq!(icon.get_attribute("src").unwrap(), "/icons/custom.svg");
    }

    #[wasm_bindgen_test]
    fn renders_label_in_span() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {}>"My Label"</IconButton>
        });
        let span = el.query_selector("button > span").unwrap().unwrap();
        assert_eq!(span.text_content().unwrap(), "My Label");
    }

    #[wasm_bindgen_test]
    fn default_classes() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {}>"X"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(!btn.class_list().contains("small"));
        assert!(!btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn small_class() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {} small=true>"X"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn primary_class() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {} primary=true>"X"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn small_and_primary_classes() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {} small=true primary=true>"X"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("small"));
        assert!(btn.class_list().contains("primary"));
    }

    #[wasm_bindgen_test]
    fn warning_class() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {} warning=true>"X"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("warning"));
    }

    #[wasm_bindgen_test]
    fn warning_and_small_classes() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {} warning=true small=true>"X"</IconButton>
        });
        let btn = el.query_selector("button").unwrap().unwrap();
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("warning"));
        assert!(btn.class_list().contains("small"));
    }

    #[wasm_bindgen_test]
    fn not_disabled_by_default() {
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {}>"X"</IconButton>
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
        let el = mount(|| view! {
            <IconButton icon="/icons/x.svg" on_click=|_| {} disabled=true>"X"</IconButton>
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
        let el = mount(move || view! {
            <IconButton icon="/icons/x.svg" on_click=move |_| clicked_clone.set(true)>"X"</IconButton>
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
}
