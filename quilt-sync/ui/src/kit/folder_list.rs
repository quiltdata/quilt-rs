//! Folder paths typed one a row, for a [`FormDialog`](super::FormDialog).
//!
//! Plain paths, never patterns: a folder keeps everything under it. The rows
//! hold what was typed, blank ones included; only [`filled`] rows are saved,
//! because a blank folder means the package root to the library. The dialog
//! owns Save, Cancel and a refusal.
//!
//! Each field is named by `aria-label`, not a [`FormControl`](super::FormControl):
//! the dialog's title says what the rows are, and a stacked label on every row
//! would say "Folder" over and over.

use leptos::prelude::*;

use super::icons;
use super::{Button, IconButton, IconButtonVariant};

stylance::import_crate_style!(style, "src/kit/folder_list.module.scss");
// The field is `TextInput`'s, drawn alike.
stylance::import_crate_style!(
    #[allow(dead_code, reason = "only the field's resting class is wanted")]
    field,
    "src/kit/text_input.module.scss"
);

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
) -> impl IntoView {
    // Whenever the caller empties the rows, as a dialog refilled on opening does.
    Effect::new(move |_| {
        if rows.with(Vec::is_empty) {
            rows.set(vec![String::new()]);
        }
    });

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
                    class=field::root
                    aria-label=format!("Folder {}", i + 1)
                    placeholder="data/raw"
                    prop:value=value
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
                />
            </div>
        }
    };

    view! {
        <div class=style::root>
            <For each=move || 0..rows.with(Vec::len) key=|i| *i children=row />
            <div class=style::add>
                <Button on_click=move |_| rows.update(|rows| rows.push(String::new()))>
                    "Add folder"
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

    fn click(el: &web_sys::Element, selector: &str) {
        el.query_selector(selector)
            .unwrap()
            .expect("the button")
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
    }

    #[wasm_bindgen_test]
    async fn no_rows_draw_one_to_type_in() {
        let list = RwSignal::new(Vec::new());
        let el = mount(move || view! { <FolderList rows=list /> });
        leptos::task::tick().await;
        assert_eq!(el.query_selector_all("input").unwrap().length(), 1);
    }

    #[wasm_bindgen_test]
    async fn rows_emptied_later_draw_one_to_type_in() {
        let list = RwSignal::new(rows(&["data/raw"]));
        let el = mount(move || view! { <FolderList rows=list /> });
        list.set(Vec::new());
        leptos::task::tick().await;
        assert_eq!(el.query_selector_all("input").unwrap().length(), 1);
    }

    #[wasm_bindgen_test]
    async fn typing_and_adding_reach_the_rows() {
        let list = RwSignal::new(rows(&["data/raw"]));
        let el = mount(move || view! { <FolderList rows=list /> });
        click(&el, "div > div:last-child > button");
        leptos::task::tick().await;
        let second: web_sys::HtmlInputElement = el
            .query_selector("[aria-label='Folder 2']")
            .unwrap()
            .expect("a second row")
            .dyn_into()
            .unwrap();
        second.set_value("results");
        second
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
        assert_eq!(list.get_untracked(), rows(&["data/raw", "results"]));
    }

    #[wasm_bindgen_test]
    async fn the_last_row_is_cleared_not_removed() {
        let list = RwSignal::new(rows(&["data/raw"]));
        let el = mount(move || view! { <FolderList rows=list /> });
        click(&el, "[aria-label='Remove folder 1']");
        leptos::task::tick().await;
        assert_eq!(list.get_untracked(), rows(&[""]));
        assert_eq!(el.query_selector_all("input").unwrap().length(), 1);
    }
}
