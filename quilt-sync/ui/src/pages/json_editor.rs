//! The metadata editor, `vanilla-jsoneditor`, over a fallback `<textarea>`.
//!
//! The glue hides the textarea's parent on mount, so give the textarea a
//! wrapper of its own. Ancestors with `overflow` crop the editor's menus.

use leptos::html;
use leptos::prelude::*;

// The boundary passes DOM elements, not id strings, so the JS registry can key
// by element identity — see the `Transition`-safety note in json-editor-glue.js.
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
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

/// Put the cursor in the editor, if it mounted. `false` when it did not, and
/// the textarea is the field the reader sees.
pub(crate) fn focus_json_editor(editor_ref: NodeRef<html::Div>) -> bool {
    editor_ref
        .get_untracked()
        .filter(|_| bundle_loaded())
        .is_some_and(|editor| focus_json_editor_js(&editor))
}

/// Put the cursor in the editor inside `within`, or in its textarea when the
/// editor did not mount.
pub(crate) fn focus_json_editor_within(within: &web_sys::Element) {
    let find = |selector: &str| {
        within
            .query_selector(selector)
            .ok()
            .flatten()
            .and_then(|el| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el).ok())
    };
    if let Some(editor) = find("[data-json-editor]").filter(|_| bundle_loaded())
        && focus_json_editor_js(&editor)
    {
        return;
    }
    if let Some(textarea) = find("textarea") {
        drop(textarea.focus());
    }
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
        <div class=class.unwrap_or("metadata") node_ref=node_ref data-json-editor></div>
    }
}
