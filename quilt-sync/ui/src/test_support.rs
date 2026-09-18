//! What the DOM tests share. `#[cfg(test)]`, so none of it reaches a binary.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Mount a view into a fresh container under `<body>` and hand back the
/// container to query.
///
/// The handle is forgotten rather than held: dropping it unmounts the view, and
/// every caller wants it to outlive the call.
pub(crate) fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
    let doc = web_sys::window().unwrap().document().unwrap();
    let container: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
    doc.body().unwrap().append_child(&container).unwrap();
    leptos::mount::mount_to(container.clone(), f).forget();
    container.into()
}

/// A promise-backed sleep — four lines over `set_timeout`, which is why the
/// crate carries no `gloo-timers`.
pub(crate) async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The innermost element drawing exactly these words — how a reader finds a
/// thing on screen, and what keeps a test off the stylesheet's class names.
pub(crate) fn element_saying(root: &web_sys::Element, text: &str) -> web_sys::HtmlElement {
    let all = root.query_selector_all("*").unwrap();
    let mut found: Option<web_sys::Element> = None;
    for i in 0..all.length() {
        let el: web_sys::Element = all.item(i).unwrap().unchecked_into();
        if el.text_content().unwrap_or_default().trim() == text {
            // A descendant follows its ancestor in document order, so the last
            // exact match is the element holding the words and nothing else.
            found = Some(el);
        }
    }
    found
        .unwrap_or_else(|| panic!("nothing says {text:?}; markup was {}", root.inner_html()))
        .unchecked_into()
}
