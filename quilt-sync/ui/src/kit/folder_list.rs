//! A list of folder paths typed one a row, with a Save.
//!
//! Plain paths, never patterns: a folder keeps everything under it. The rows
//! hold what was typed, blank ones included; only [`filled`] rows are saved,
//! because a blank folder means the package root to the library.
//!
//! Each field is named by `aria-label`, not a [`FormControl`](super::FormControl):
//! the list sits under its group's label, and a stacked label on every row
//! would say "Folder" over and over.

use leptos::prelude::*;

use super::icons;
use super::state_label::StateTone;
use super::{Button, IconButton, IconButtonVariant};

stylance::import_crate_style!(style, "src/kit/folder_list.module.scss");
// The field is `TextInput`'s, drawn alike.
stylance::import_crate_style!(field, "src/kit/text_input.module.scss");

/// The rows worth saving: those with something in them, in order.
#[must_use]
pub fn filled(rows: &[String]) -> Vec<String> {
    rows.iter()
        .filter(|row| !row.trim().is_empty())
        .cloned()
        .collect()
}

#[component]
pub fn FolderList(
    /// The rows as typed. At least one row is always drawn.
    rows: RwSignal<Vec<String>>,
    /// Gets the [`filled`] rows. Never called with none, nor while disabled.
    on_save: Callback<Vec<String>>,
    /// Why the last save was refused, under the rows.
    #[prop(optional, into)]
    error: MaybeProp<String>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// The save is running: Save spins and the rows are sealed.
    #[prop(optional, into)]
    saving: MaybeProp<bool>,
) -> impl IntoView {
    if rows.with_untracked(Vec::is_empty) {
        rows.set(vec![String::new()]);
    }
    let sealed =
        Signal::derive(move || disabled.get().unwrap_or(false) || saving.get().unwrap_or(false));
    let empty = Signal::derive(move || rows.with(|rows| filled(rows).is_empty()));
    let invalid = Signal::derive(move || error.get().is_some());

    let row = move |i: usize| {
        let value = move || rows.with(|rows| rows.get(i).cloned().unwrap_or_default());
        // The last row is cleared rather than removed, so there is always one to type in.
        let remove = move |_| {
            rows.update(|rows| {
                if rows.len() > 1 {
                    rows.remove(i);
                } else {
                    rows[0].clear();
                }
            });
        };
        view! {
            <div class=style::row>
                <input
                    type="text"
                    class=move || {
                        if invalid.get() {
                            format!("{} {}", field::root, field::invalid)
                        } else {
                            field::root.to_string()
                        }
                    }
                    aria-label=format!("Folder {}", i + 1)
                    aria-invalid=move || invalid.get().then_some("true")
                    placeholder="data/raw"
                    prop:value=value
                    prop:disabled=move || sealed.get()
                    on:input=move |ev| {
                        let typed = event_target_value(&ev);
                        rows.update(|rows| {
                            if let Some(row) = rows.get_mut(i) {
                                *row = typed;
                            }
                        });
                    }
                />
                <IconButton
                    icon=icons::x()
                    aria_label=format!("Remove folder {}", i + 1)
                    on_click=remove
                    variant=IconButtonVariant::Invisible
                    disabled=sealed
                />
            </div>
        }
    };

    view! {
        <div class=style::root>
            <For each=move || 0..rows.with(Vec::len) key=|i| *i children=row />
            <Show when=move || error.get().is_some()>
                <p class=style::error>
                    {StateTone::Danger.glyph()}
                    <span>{move || error.get().unwrap_or_default()}</span>
                </p>
            </Show>
            <div class=style::actions>
                <Button
                    on_click=move |_| rows.update(|rows| rows.push(String::new()))
                    disabled=sealed
                >
                    "Add folder"
                </Button>
                <Button
                    on_click=move |_| on_save.run(rows.with_untracked(|rows| filled(rows)))
                    disabled=Signal::derive(move || disabled.get().unwrap_or(false) || empty.get())
                    loading=saving
                >
                    "Save"
                </Button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{FolderList, filled};
    use crate::test_support::mount;
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn rows(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn blank_rows_are_not_saved() {
        assert_eq!(
            filled(&rows(&["data/raw", "", "  ", "results"])),
            rows(&["data/raw", "results"])
        );
        assert_eq!(filled(&rows(&["", " "])), Vec::<String>::new());
    }

    fn button(el: &web_sys::Element, words: &str) -> web_sys::HtmlButtonElement {
        let buttons = el.query_selector_all("button").unwrap();
        (0..buttons.length())
            .filter_map(|i| buttons.item(i))
            .find(|b| b.text_content().unwrap_or_default().contains(words))
            .expect("the button")
            .dyn_into()
            .unwrap()
    }

    /// Nothing filled in has nothing to save: Save would store no folders,
    /// which the library reads as files picked by hand.
    #[wasm_bindgen_test]
    fn save_waits_for_a_folder() {
        let list = RwSignal::new(Vec::new());
        let el = mount(move || {
            view! { <FolderList rows=list on_save=Callback::new(|_| ()) /> }
        });
        assert_eq!(el.query_selector_all("input").unwrap().length(), 1);
        assert!(button(&el, "Save").disabled());
    }

    #[wasm_bindgen_test]
    async fn save_sends_the_filled_rows() {
        let list = RwSignal::new(rows(&["data/raw", ""]));
        let saved = RwSignal::new(None);
        let el = mount(move || {
            view! {
                <FolderList
                    rows=list
                    on_save=Callback::new(move |folders| saved.set(Some(folders)))
                />
            }
        });
        button(&el, "Save").click();
        leptos::task::tick().await;
        assert_eq!(saved.get_untracked(), Some(rows(&["data/raw"])));
    }

    #[wasm_bindgen_test]
    fn a_disabled_list_does_not_save() {
        let list = RwSignal::new(rows(&["data/raw"]));
        let el = mount(move || {
            view! { <FolderList rows=list on_save=Callback::new(|_| ()) disabled=true /> }
        });
        assert!(button(&el, "Save").disabled());
    }

    #[wasm_bindgen_test]
    async fn the_last_row_is_cleared_not_removed() {
        let list = RwSignal::new(rows(&["data/raw"]));
        let el = mount(move || {
            view! { <FolderList rows=list on_save=Callback::new(|_| ()) /> }
        });
        el.query_selector("[aria-label='Remove folder 1']")
            .unwrap()
            .expect("the remove button")
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        leptos::task::tick().await;
        assert_eq!(list.get_untracked(), rows(&[""]));
        assert_eq!(el.query_selector_all("input").unwrap().length(), 1);
    }
}
