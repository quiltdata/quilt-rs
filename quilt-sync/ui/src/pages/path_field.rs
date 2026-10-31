//! A folder typed by hand, where the engine has no desktop to show a picker
//! (a browser never gives a page an absolute path). It says whether the path
//! is a folder, and offers the next part of it: Tab or → takes it.

use leptos::prelude::*;

use crate::commands;
use crate::kit::{FormControl, TextInput};

stylance::import_crate_style!(style, "src/pages/path_field.module.css");

/// What the engine said of the path as typed.
#[derive(Clone, Default, PartialEq, Eq)]
struct Checked {
    path: String,
    check: commands::PathCheck,
    offered: Vec<String>,
}

#[component]
pub fn PathField(
    #[prop(into)] label: String,
    value: RwSignal<String>,
    /// A folder that isn't there yet is fine: the caller makes it.
    #[prop(optional)]
    may_be_new: bool,
    #[prop(optional, into)] placeholder: String,
) -> impl IntoView {
    let checked = RwSignal::new(Checked::default());
    Effect::new(move |_| {
        let path = value.get();
        if path.is_empty() {
            checked.set(Checked::default());
            return;
        }
        leptos::task::spawn_local(async move {
            let check = commands::check_path(path.clone()).await.unwrap_or_default();
            let offered = commands::complete_path(path.clone())
                .await
                .unwrap_or_default();
            // Only the answer for what's typed now: typing on overtakes it.
            if value.try_get_untracked().as_ref() == Some(&path) {
                checked.try_set(Checked {
                    path,
                    check,
                    offered,
                });
            }
        });
    });
    // Nothing to say of an empty field, nor of an answer for an older text.
    let current = move || {
        let now = value.get();
        checked.with(|c| (!now.is_empty() && c.path == now).then(|| c.clone()))
    };
    let next = move || current().and_then(|c| c.offered.into_iter().find(|offer| *offer != c.path));
    let error = Signal::derive(move || {
        let c = current()?;
        match (c.check.exists, c.check.is_dir) {
            (true, true) => None,
            (true, false) => Some("This is a file, not a folder.".to_owned()),
            (false, _) if may_be_new => None,
            (false, _) => Some("There is no folder here.".to_owned()),
        }
    });
    let hint = move || {
        if let Some(next) = next() {
            return format!("Tab: {next}");
        }
        match current() {
            Some(c) if may_be_new && !c.check.exists => {
                "QuiltSync will make this folder.".to_owned()
            }
            _ => String::new(),
        }
    };
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let takes = match ev.key().as_str() {
            "Tab" => !ev.shift_key(),
            // Only with the caret at the end, where → would do nothing.
            "ArrowRight" => caret_at_end(&ev),
            _ => false,
        };
        if takes && let Some(next) = next() {
            ev.prevent_default();
            value.set(next);
        }
    };

    view! {
        <FormControl
            label=label
            error=error
            control=move |id| {
                view! {
                    <div on:keydown=on_keydown>
                        <TextInput id=id value=value placeholder=placeholder />
                        <p class=style::hint aria-live="polite">{hint}</p>
                    </div>
                }
                    .into_any()
            }
        />
    }
}

fn caret_at_end(ev: &web_sys::KeyboardEvent) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|input| {
            let len = u32::try_from(input.value().encode_utf16().count()).unwrap_or(u32::MAX);
            input.selection_start().ok().flatten() == Some(len)
        })
}

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;
    use crate::engine::fake::FakeEngine;
    use crate::test_support::{mount, sleep_ms, unmount_earlier};

    fn input(root: &web_sys::Element) -> web_sys::HtmlInputElement {
        root.query_selector("input")
            .unwrap()
            .expect("the field")
            .unchecked_into()
    }

    fn type_into(field: &web_sys::HtmlInputElement, text: &str) {
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    /// Answers the field's two asks for what's typed: a check, then the
    /// folders on offer.
    async fn answer(engine: &FakeEngine, check: serde_json::Value, offered: serde_json::Value) {
        sleep_ms(0).await;
        if engine.requests().is_empty() {
            // The first ask opened the socket; it waits for the hello.
            engine.hello();
            sleep_ms(0).await;
        }
        let requests = engine.requests();
        let last = requests.len();
        assert_eq!(requests[last - 1]["cmd"], "check_path");
        engine.reply(&requests[last - 1]["id"], &check);
        sleep_ms(0).await;
        let requests = engine.requests();
        assert_eq!(requests[requests.len() - 1]["cmd"], "complete_path");
        engine.reply(&requests[requests.len() - 1]["id"], &offered);
        sleep_ms(0).await;
    }

    #[wasm_bindgen_test]
    async fn tab_takes_the_next_part_and_a_file_is_not_a_folder() {
        unmount_earlier();
        let engine = FakeEngine::install();
        let value = RwSignal::new(String::new());
        let root = mount(move || view! { <PathField label="Folder" value=value /> });
        sleep_ms(0).await;
        let field = input(&root);

        type_into(&field, "/mnt/da");
        answer(
            &engine,
            serde_json::json!({"exists": false, "isDir": false}),
            serde_json::json!(["/mnt/data/"]),
        )
        .await;
        let text = root.text_content().unwrap_or_default();
        assert!(text.contains("Tab: /mnt/data/"), "{text}");
        assert!(text.contains("There is no folder here."), "{text}");

        let tab = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &{
            let init = web_sys::KeyboardEventInit::new();
            init.set_key("Tab");
            init.set_bubbles(true);
            init.set_cancelable(true);
            init
        })
        .unwrap();
        field.dispatch_event(&tab).unwrap();
        assert_eq!(value.get_untracked(), "/mnt/data/");
        assert!(tab.default_prevented(), "Tab stays in the field");

        answer(
            &engine,
            serde_json::json!({"exists": true, "isDir": true}),
            serde_json::json!([]),
        )
        .await;
        let text = root.text_content().unwrap_or_default();
        assert!(!text.contains("There is no folder here."), "{text}");

        type_into(&field, "/mnt/data/notes.txt");
        answer(
            &engine,
            serde_json::json!({"exists": true, "isDir": false}),
            serde_json::json!([]),
        )
        .await;
        let text = root.text_content().unwrap_or_default();
        assert!(text.contains("This is a file, not a folder."), "{text}");
        unmount_earlier();
    }

    #[wasm_bindgen_test]
    async fn an_empty_field_says_nothing() {
        unmount_earlier();
        let _engine = FakeEngine::install();
        let value = RwSignal::new(String::new());
        let root = mount(move || view! { <PathField label="Folder" value=value /> });
        sleep_ms(0).await;
        let text = root.text_content().unwrap_or_default();
        assert!(!text.contains("There is no folder here."), "{text}");
        unmount_earlier();
    }

    #[wasm_bindgen_test]
    async fn a_new_folder_is_fine_where_the_caller_makes_it() {
        unmount_earlier();
        let engine = FakeEngine::install();
        let value = RwSignal::new(String::new());
        let root =
            mount(move || view! { <PathField label="Folder" value=value may_be_new=true /> });
        sleep_ms(0).await;
        type_into(&input(&root), "/home/a/QuiltSync");
        answer(
            &engine,
            serde_json::json!({"exists": false, "isDir": false}),
            serde_json::json!([]),
        )
        .await;
        let text = root.text_content().unwrap_or_default();
        assert!(text.contains("QuiltSync will make this folder."), "{text}");
        assert!(!text.contains("There is no folder here."), "{text}");
        unmount_earlier();
    }
}
