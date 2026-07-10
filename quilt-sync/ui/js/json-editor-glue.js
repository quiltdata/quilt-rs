// Thin wrapper around vanilla-jsoneditor that exposes the three functions
// consumed by the Leptos commit page via wasm-bindgen.
//
// Bundled by esbuild during `trunk build` (see Trunk.toml hook).
import { createJSONEditor } from "vanilla-jsoneditor/standalone.js";

// Registry keyed by the editor's DOM element, NOT an id string. During the
// commit page's keep-alive subtree swap (the outer `Transition`), an old and a
// new editor container are alive at once; keying by id — or resolving via
// `document.getElementById` — would be ambiguous by construction and let a
// replaced subtree's cleanup kill its successor's editor. Element identity never
// is. A WeakMap needs no manual eviction to avoid leaks, and `destroy` still
// deletes its entry so the same element is never destroyed twice.
const editors = new WeakMap();

function mountEditor(target, textarea, initialValue) {
  if (!target || editors.has(target)) return;
  // Hide the textarea fallback belonging to THIS dialog. The textarea element
  // is passed in (not looked up by id) so we always hide the current dialog's
  // fallback, never another dialog's during a swap.
  if (textarea && textarea.parentElement)
    textarea.parentElement.style.display = "none";
  const editor = createJSONEditor({
    target,
    props: {
      content: { text: initialValue },
      onChange: (updatedContent) => {
        if (!textarea) return;
        if (updatedContent.json !== undefined)
          textarea.value = JSON.stringify(updatedContent.json);
        else textarea.value = updatedContent.text ?? "";
        // Mirror the edit into the (hidden) textarea AND notify Leptos: a
        // programmatic `value` assignment does not fire an `input` event, so
        // dispatch one so the commit page's live-validation signal tracks
        // metadata edits made through the JSON editor. The textarea is captured
        // from this closure, so the write always targets the CURRENT dialog's
        // textarea, never the first `#metadata` match in the document.
        textarea.dispatchEvent(new Event("input", { bubbles: true }));
      },
      navigationBar: false,
    },
  });
  editors.set(target, editor);
}

window.__createJsonEditor = function (target, textarea, initialValue) {
  mountEditor(target, textarea, initialValue);
};

window.__getJsonEditorValue = function (target) {
  const editor = target && editors.get(target);
  if (!editor) return "";
  const content = editor.get();
  if (content.json !== undefined) return JSON.stringify(content.json);
  return content.text ?? "";
};

window.__destroyJsonEditor = function (target) {
  const editor = target && editors.get(target);
  if (editor) {
    editor.destroy();
    editors.delete(target);
  }
};
