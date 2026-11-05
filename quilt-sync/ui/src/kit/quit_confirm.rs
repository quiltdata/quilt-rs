//! The quit prompt, as the kit's [`ConfirmDialog`]: the question that stands
//! between a quit and an interrupted apply.
//!
//! *Quit anyway* is the Danger verb and *Stay* the safe answer, so focus lands
//! on *Stay* and a stray Return does not quit. Every way the dialog closes
//! other than the verb answers *Stay*: the button, and Escape, which here is an
//! answer and not only a dismissal, since abandoning the quit is one of the two
//! choices. The backdrop does not close it, as no kit dialog's does.
//!
//! Presentation only: the caller sends the answers to the backend.

use leptos::prelude::*;

use super::ConfirmDialog;
use super::Submit;

pub const QUIT_TITLE: &str = "Still syncing";
pub const QUIT_CONSEQUENCE: &str = "QuiltSync is writing files right now. Quitting will \
     interrupt it and leave this package part-way between two revisions.";

#[component]
pub fn QuitConfirm(
    /// The caller's, in both directions, as [`ConfirmDialog`]'s own.
    open: RwSignal<bool>,
    #[prop(into)] on_quit: Callback<()>,
    #[prop(into)] on_stay: Callback<()>,
) -> impl IntoView {
    // Set by the verb before the dialog closes, so that close is not read as Stay.
    let quitting = StoredValue::new(false);

    Effect::new(move |was_open: Option<bool>| {
        let now = open.get();
        if was_open == Some(true) && !now {
            if quitting.get_value() {
                quitting.set_value(false);
            } else {
                on_stay.run(());
            }
        }
        now
    });

    view! {
        <ConfirmDialog
            open=open
            title=QUIT_TITLE
            consequence=QUIT_CONSEQUENCE
            cancel="Stay"
            confirm=Submit::new(
                "Quit anyway",
                move || {
                    quitting.set_value(true);
                    on_quit.run(());
                    async { Ok(()) }
                },
            )
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::button_saying;
    use crate::test_support::mount;
    use crate::test_support::sleep_ms;
    use wasm_bindgen_test::*;

    /// How often each answer was given.
    struct Answers {
        quit: RwSignal<u32>,
        stay: RwSignal<u32>,
    }

    fn prompt(open: RwSignal<bool>) -> (web_sys::Element, Answers) {
        let quit = RwSignal::new(0);
        let stay = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <QuitConfirm
                    open=open
                    on_quit=move |()| quit.update(|n| *n += 1)
                    on_stay=move |()| stay.update(|n| *n += 1)
                />
            }
        });
        (el, Answers { quit, stay })
    }

    async fn settle() {
        sleep_ms(0).await;
        leptos::task::tick().await;
    }

    #[wasm_bindgen_test]
    async fn focus_lands_on_stay() {
        let open = RwSignal::new(false);
        let (el, _) = prompt(open);
        open.set(true);
        settle().await;
        let stay = button_saying(&el, "Stay");
        let focused = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
            .expect("something has focus");
        assert!(
            stay.is_same_node(Some(&focused)),
            "focus is on {:?}",
            focused.text_content()
        );
    }

    #[wasm_bindgen_test]
    async fn quit_anyway_quits_and_does_not_also_stay() {
        let open = RwSignal::new(false);
        let (el, answers) = prompt(open);
        open.set(true);
        settle().await;
        let verb = button_saying(&el, "Quit anyway");
        assert!(
            verb.class_name().contains("danger"),
            "the verb is Danger: {}",
            verb.class_name()
        );
        verb.click();
        settle().await;
        assert_eq!(answers.quit.get_untracked(), 1);
        assert_eq!(answers.stay.get_untracked(), 0);
        assert!(!open.get_untracked(), "and the prompt closes");
    }

    #[wasm_bindgen_test]
    async fn stay_answers_stay() {
        let open = RwSignal::new(false);
        let (el, answers) = prompt(open);
        open.set(true);
        settle().await;
        button_saying(&el, "Stay").click();
        settle().await;
        assert_eq!(answers.stay.get_untracked(), 1);
        assert_eq!(answers.quit.get_untracked(), 0);
    }

    /// Escape closes the dialog, and a close that is not the verb is Stay.
    #[wasm_bindgen_test]
    async fn escape_answers_stay() {
        let open = RwSignal::new(false);
        let (el, answers) = prompt(open);
        open.set(true);
        settle().await;
        let dialog: web_sys::HtmlDialogElement = wasm_bindgen::JsCast::unchecked_into(
            el.query_selector("dialog").unwrap().expect("the dialog"),
        );
        // What the UA does on Escape once nothing prevents its `cancel`.
        dialog.close();
        settle().await;
        assert!(!open.get_untracked());
        assert_eq!(answers.stay.get_untracked(), 1);
        assert_eq!(answers.quit.get_untracked(), 0);
    }
}
