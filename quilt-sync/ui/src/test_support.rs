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
