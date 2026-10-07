//! The metadata editor: `vanilla-jsoneditor`, mounted over a fallback
//! `<textarea>` through `js/json-editor-glue.js`.
//!
//! Shared by the commit page and its v2. The glue hides the textarea's
//! **parent** once the editor mounts, so a caller gives the textarea a wrapper
//! of its own: on v1 that is its `<p>`, on v2 a bare `div` — never a
//! `FormControl`, whose label and error must stay.
//!
//! The editor draws its context menu and its dropdowns inside its own box, so
//! an ancestor with `overflow: hidden` or `auto` crops them. v1's two columns
//! needed a scrolling column; the v2 page is one column and gives the editor no
//! clipping ancestor.

use leptos::html;
use leptos::prelude::*;

// The boundary passes DOM elements, not id strings, so the JS registry can key
// by element identity — see the `Transition`-safety note in json-editor-glue.js.
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window"], js_name = "__getJsonEditorValue")]
    fn get_json_editor_value_js(target: &web_sys::HtmlElement) -> String;

    #[wasm_bindgen(js_namespace = ["window"], js_name = "__createJsonEditor")]
    fn create_json_editor_js(
        target: &web_sys::HtmlElement,
        textarea: &web_sys::HtmlElement,
        initial_value: &str,
    );

    #[wasm_bindgen(js_namespace = ["window"], js_name = "__destroyJsonEditor")]
    fn destroy_json_editor_js(target: &web_sys::HtmlElement);
}

/// The glue has run: `index.html` and `gallery.html` load it, a test runner's
/// document does not.
fn bundle_loaded() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::get(&window, &"__createJsonEditor".into()).is_ok_and(|f| f.is_function())
    })
}

/// Read the committed metadata at submit: the editor's live value, or the
/// textarea when the editor never mounted or is empty.
pub(crate) fn get_json_editor_value(
    editor_ref: NodeRef<html::Div>,
    textarea_ref: NodeRef<html::Textarea>,
) -> String {
    if let Some(editor) = editor_ref.get_untracked().filter(|_| bundle_loaded()) {
        let value = get_json_editor_value_js(&editor);
        if !value.is_empty() {
            return value;
        }
    }
    textarea_ref
        .get_untracked()
        .map(|ta| ta.value())
        .unwrap_or_default()
}

#[component]
pub(crate) fn JsonEditor(
    node_ref: NodeRef<html::Div>,
    textarea_ref: NodeRef<html::Textarea>,
    initial_value: String,
    /// The editor box's class. v1's `metadata` by default.
    #[prop(optional)]
    class: Option<&'static str>,
) -> impl IntoView {
    // Mount needs both the editor div and the textarea. The div is DOM-ordered
    // after the textarea, so the div's `on_load` alone would suffice; both are
    // wired so the mount fires on whichever ref lands last (`on_load` is
    // effect-based and fires even for a ref already loaded when registered),
    // and the guard makes the redundant call a no-op.
    let mounted = StoredValue::new(false);
    let init = StoredValue::new(initial_value);
    let try_mount = move || {
        if mounted.get_value() {
            return;
        }
        let (Some(editor), Some(textarea)) =
            (node_ref.get_untracked(), textarea_ref.get_untracked())
        else {
            return;
        };
        mounted.set_value(true);
        // The bundle is a page's `<script>`, so a document without it — the
        // test runner's — keeps the textarea rather than trapping on a call
        // to a function that is not there.
        if !bundle_loaded() {
            return;
        }
        init.with_value(|v| create_json_editor_js(&editor, &textarea, v));
    };
    node_ref.on_load(move |_| try_mount());
    textarea_ref.on_load(move |_| try_mount());

    on_cleanup(move || {
        if !bundle_loaded() {
            return;
        }
        if let Some(editor) = node_ref.get_untracked() {
            destroy_json_editor_js(&editor);
        }
    });

    view! {
        <div class=class.unwrap_or("metadata") node_ref=node_ref></div>
    }
}
