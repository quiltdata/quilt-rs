// Thin wrapper around vanilla-jsoneditor that exposes the three functions
// consumed by the Leptos commit page via wasm-bindgen.
//
// Bundled by esbuild during `trunk build` (see Trunk.toml hook).
import { createJSONEditor } from "vanilla-jsoneditor/standalone.js";

const editors = new Map();

function mountEditor(targetId, initialValue) {
  if (editors.has(targetId)) return;
  const target = document.getElementById(targetId);
  if (!target) return;
  // Hide the textarea fallback
  const textarea = document.getElementById("metadata");
  if (textarea && textarea.parentElement)
    textarea.parentElement.style.display = "none";
  const editor = createJSONEditor({
    target,
    props: {
      content: { text: initialValue },
      onChange: (updatedContent) => {
        const ta = document.getElementById("metadata");
        if (ta) {
          if (updatedContent.json !== undefined)
            ta.value = JSON.stringify(updatedContent.json);
          else ta.value = updatedContent.text ?? "";
        }
      },
      navigationBar: false,
    },
  });
  editors.set(targetId, editor);
}

window.__createJsonEditor = function (targetId, initialValue) {
  mountEditor(targetId, initialValue);
};

window.__getJsonEditorValue = function (targetId) {
  const editor = editors.get(targetId);
  if (!editor) return "";
  const content = editor.get();
  if (content.json !== undefined) return JSON.stringify(content.json);
  return content.text ?? "";
};

window.__destroyJsonEditor = function (targetId) {
  const editor = editors.get(targetId);
  if (editor) {
    editor.destroy();
    editors.delete(targetId);
  }
};
