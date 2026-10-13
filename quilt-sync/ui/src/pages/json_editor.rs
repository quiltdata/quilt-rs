//! The metadata editor, `vanilla-jsoneditor`, over a fallback `<textarea>`.
//! Shared by both commit pages.
//!
//! The glue hides the textarea's parent on mount, so give the textarea a
//! wrapper of its own. Ancestors with `overflow` crop the editor's menus.

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

    #[wasm_bindgen(js_namespace = ["window"], js_name = "__focusJsonEditor")]
    fn focus_json_editor_js(target: &web_sys::HtmlElement) -> bool;

    #[wasm_bindgen(js_namespace = ["window"], js_name = "__destroyJsonEditor")]
    fn destroy_json_editor_js(target: &web_sys::HtmlElement);
}

/// The test runner's document has no bundle.
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

/// Put the cursor in the editor, if it mounted. `false` when it did not, and
/// the textarea is the field the reader sees.
pub(crate) fn focus_json_editor(editor_ref: NodeRef<html::Div>) -> bool {
    editor_ref
        .get_untracked()
        .filter(|_| bundle_loaded())
        .is_some_and(|editor| focus_json_editor_js(&editor))
}

#[component]
pub(crate) fn JsonEditor(
    node_ref: NodeRef<html::Div>,
    textarea_ref: NodeRef<html::Textarea>,
    initial_value: String,
    /// `metadata` by default.
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
        // Without the bundle, keep the textarea rather than trap.
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
