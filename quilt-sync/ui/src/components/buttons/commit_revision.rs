use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, base::cta::ButtonCta};

const KIND: ButtonKind = ButtonKind::CommitRevision;

#[component]
pub fn CommitRevision(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] busy: MaybeProp<bool>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    #[prop(optional, into, default = MaybeProp::from(true))] primary: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled =
        Signal::derive(move || busy.get().unwrap_or(false) || disabled.get().unwrap_or(false));
    view! {
        <ButtonCta icon=KIND.icon() on_click=UnsyncCallback::new(on_click) primary=primary disabled=is_disabled>
            {move || if busy.get().unwrap_or(false) { "Committing\u{2026}" } else { KIND.label() }}
        </ButtonCta>
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

    fn get_button(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        el.query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    #[wasm_bindgen_test]
    fn not_disabled_by_default() {
        let el = mount(|| view! { <CommitRevision on_click=|_| {} /> });
        assert!(!get_button(&el).disabled());
    }

    #[wasm_bindgen_test]
    fn disabled_when_busy() {
        let el = mount(|| view! { <CommitRevision on_click=|_| {} busy=true /> });
        assert!(get_button(&el).disabled());
    }

    #[wasm_bindgen_test]
    fn disabled_when_disabled_prop() {
        let el = mount(|| view! { <CommitRevision on_click=|_| {} disabled=true /> });
        assert!(get_button(&el).disabled());
    }

    #[wasm_bindgen_test]
    fn shows_committing_label_when_busy() {
        let el = mount(|| view! { <CommitRevision on_click=|_| {} busy=true /> });
        assert_eq!(el.text_content().unwrap().trim(), "Committing\u{2026}");
    }

    #[wasm_bindgen_test]
    fn shows_commit_label_when_disabled_but_not_busy() {
        let el = mount(|| view! { <CommitRevision on_click=|_| {} disabled=true /> });
        assert_eq!(el.text_content().unwrap().trim(), KIND.label());
    }
}
