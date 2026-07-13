// Bundled by esbuild during `trunk build` (see Trunk.toml hook).
import { createJSONEditor } from "vanilla-jsoneditor/standalone.js";

// Keyed by DOM element, never by id: the commit page's `Transition` keeps an
// old and new editor container alive at once, so an id (or `getElementById`)
// could resolve to the wrong one and let a stale cleanup destroy the live
// editor. Element identity can't. WeakMap avoids manual eviction.
const editors = new WeakMap();

function mountEditor(target, textarea, initialValue) {
  if (!target || editors.has(target)) return;
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
        // Assigning `.value` does not fire `input`; dispatch one so the
        // commit page's live-validation signal sees edits made in the editor.
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
