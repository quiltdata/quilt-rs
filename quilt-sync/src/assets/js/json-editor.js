import { createJSONEditor } from "vanilla-jsoneditor/standalone.js";

const editors = new Map();

function mountEditor(targetId) {
  if (editors.has(targetId)) return;
  const target = document.getElementById(targetId);
  if (!target) return;

  const initialValue = target.dataset.initial || "";

  // Hide the textarea field
  const textarea = document.getElementById("metadata");
  if (textarea && textarea.parentElement) {
    textarea.parentElement.style.display = "none";
  }

  const editor = createJSONEditor({
    target,
    props: {
      content: { text: initialValue },
      onChange: (updatedContent) => {
        const ta = document.getElementById("metadata");
        if (ta) {
          if (updatedContent.json !== undefined) {
            ta.value = JSON.stringify(updatedContent.json);
          } else {
            ta.value = updatedContent.text ?? "";
          }
        }
      },
      navigationBar: false,
    },
  });
  editors.set(targetId, editor);
}

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

// Watch for the editor target div to appear in the DOM
const observer = new MutationObserver(() => {
  mountEditor("metadata-editor");
});
observer.observe(document.documentElement, {
  childList: true,
  subtree: true,
});

// Also try immediately in case the element already exists
mountEditor("metadata-editor");
