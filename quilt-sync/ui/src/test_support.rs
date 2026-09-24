//! What the DOM tests share. `#[cfg(test)]`, so none of it reaches a binary.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

thread_local! {
    /// Every view `mount` has drawn, still mounted, with the owner above it.
    static MOUNTED: std::cell::RefCell<Vec<(Owner, Box<dyn std::any::Any>)>> =
        std::cell::RefCell::default();
}

/// Mount a view into a fresh container under `<body>` and hand back the
/// container to query.
///
/// The handle is kept rather than dropped: dropping it unmounts the view, and
/// every caller wants it to outlive the call. [`unmount_earlier`] is the one
/// way it goes.
pub(crate) fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
    let doc = web_sys::window().unwrap().document().unwrap();
    let container: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
    doc.body().unwrap().append_child(&container).unwrap();
    // An owner above the handle's own, because something the view spawns can
    // keep that one alive past its handle; this one is cleaned up by hand.
    let owner = Owner::new();
    let handle = owner.with(|| leptos::mount::mount_to(container.clone(), f));
    MOUNTED.with(|m| m.borrow_mut().push((owner, Box::new(handle))));
    container.into()
}

/// Unmount every view earlier tests mounted. Each `Router` listens for link
/// clicks on the window and the first one registered takes the click, so a
/// test that clicks a link must be the only router left.
pub(crate) fn unmount_earlier() {
    let earlier = MOUNTED.with(|m| std::mem::take(&mut *m.borrow_mut()));
    for (owner, handle) in earlier {
        owner.cleanup();
        drop(handle);
    }
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

/// The button whose text is exactly these words, as a reader finds it.
pub(crate) fn button_saying(root: &web_sys::Element, text: &str) -> web_sys::HtmlButtonElement {
    let all = root.query_selector_all("button").unwrap();
    (0..all.length())
        .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
        .find(|b| b.text_content().unwrap_or_default().trim() == text)
        .unwrap_or_else(|| panic!("no button says {text:?}; markup was {}", root.inner_html()))
        .unchecked_into()
}
