//! A [`Dialog`](super::Dialog) that asks one question before a destructive command runs.
//!
//! # The confirmation `ActionTone::Danger` promises
//!
//! [`ActionTone::Danger`](super::ActionTone)'s doc reads: *always followed by a
//! confirmation — the menu picks the command, the dialog accepts the consequence.* This is
//! that dialog. It says the consequence in one sentence, offers `Cancel` and the verb, and
//! runs the verb's action itself.
//!
//! # `FormDialog`'s machinery, not a copy of it
//!
//! Everything after the verb is pressed is [`FormDialog`](super::FormDialog)'s contract,
//! through the `submission` module both share: the action is owned and awaited; both
//! buttons are refused and Escape is held while it runs; a refusal is drawn inside as a
//! `Critical` banner and the dialog stays open; success closes it; an outcome from an
//! opening that has since closed is dropped whole. What differs is the body — a sentence,
//! not fields — and that difference is why this is not `FormDialog` with prose as
//! children: there is nothing to submit, so there is no `<form>`, no `type=submit` and no
//! `form=` association, and the primary is a plain button whose click runs the action.
//!
//! # Cancel is the answer the dialog steers toward
//!
//! The primary is `Danger`, not `Primary`: a region's primary is the affordance it steers
//! you toward, and a confirmation steers you *away* from its verb. So focus lands on
//! `Cancel` — `showModal()` hands focus to the first `autofocus` inside the dialog — and a
//! stray Return does nothing destructive. Cancel is also first in the footer and the verb
//! last, as every footer on this platform, so a UA that focuses the first control lands
//! on Cancel too.
//!
//! # Escape is a dismissal here, and an answer in the quit prompt
//!
//! Escape answers Cancel, which is `Dialog`'s own default: the UA fires `cancel` and
//! closes, and closing chooses nothing. The quit prompt's Escape is different in kind —
//! staying is one of its two answers — so the chrome's claim that it is the one popup
//! whose Escape *answers* stays true. Escape is held only while the action runs, as
//! `FormDialog` holds it.

use leptos::prelude::*;

use super::Button;
use super::ButtonVariant;
use super::Dialog;
use super::Submit;
use super::submission::Submission;

stylance::import_crate_style!(style, "src/kit/confirm_dialog.module.scss");

#[component]
pub fn ConfirmDialog(
    /// The caller's, in both directions — see [`Dialog`]'s own `open`.
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    /// The one sentence saying what the verb does that cannot be taken back — `Removes
    /// the working files, including edits never committed.` A `String` and not children:
    /// one sentence is the contract, and a confirmation with more to say is a `FormDialog`.
    #[prop(into)]
    consequence: String,
    /// The verb and what it does. The label goes on the `Danger` primary; the action's
    /// `Err` becomes the banner.
    confirm: Submit,
) -> impl IntoView {
    let Submit { label, action } = confirm;
    let submission = Submission::new(open);
    let busy = submission.busy;
    let banner = submission.banner();

    let footer = view! {
        // `autofocus`, so `showModal()` lands on the safe answer; first, so a UA that
        // focuses the first control lands there as well.
        <Button autofocus=true disabled=busy on_click=move |_| open.set(false)>
            "Cancel"
        </Button>
        <Button
            variant=ButtonVariant::Danger
            loading=busy
            on_click=move |_| submission.run(&action)
        >
            {label}
        </Button>
    }
    .into_any();

    view! {
        <Dialog open=open title=title held=busy footer=footer>
            {banner}
            <p class=style::consequence>{consequence}</p>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::element_saying;
    use crate::test_support::mount;
    use crate::test_support::sleep_ms;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    // Not repeated here: an outcome from a closed session being dropped is `Submission`'s
    // rule, and `FormDialog`'s `an_outcome_cannot_reach_the_session_after_it` exercises it
    // end to end. This dialog adds no path of its own to it.

    const SENTENCE: &str = "Removes the working files, including edits never committed.";

    /// The dialog under test. `outcome` is what the verb's action answers after
    /// `delay_ms`, or at once for `0`; `ran` counts how often it was asked.
    fn confirm(
        open: RwSignal<bool>,
        ran: RwSignal<u32>,
        delay_ms: i32,
        outcome: Result<(), &'static str>,
    ) -> web_sys::Element {
        mount(move || {
            view! {
                <ConfirmDialog
                    open=open
                    title="Remove package"
                    consequence=SENTENCE
                    confirm=Submit::new(
                        "Remove",
                        move || async move {
                            ran.update(|n| *n += 1);
                            // A zero timer is not instant: it would land after `settle`'s own.
                            if delay_ms > 0 {
                                sleep_ms(delay_ms).await;
                            }
                            outcome.map_err(String::from)
                        },
                    )
                />
            }
        })
    }

    fn dialog(el: &web_sys::Element) -> web_sys::Element {
        el.query_selector("dialog").unwrap().expect("the dialog")
    }

    fn button(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        element_saying(el, label)
            .closest("button")
            .unwrap()
            .expect("the button holding those words")
            .dyn_into()
            .unwrap()
    }

    /// The footer's buttons, in document order, by what they say.
    fn footer_labels(el: &web_sys::Element) -> Vec<String> {
        let all = el.query_selector_all("dialog button").unwrap();
        (0..all.length())
            .filter_map(|i| all.item(i).unwrap().text_content())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// A task boundary and a tick: the action settles on the event loop, then the DOM.
    async fn settle() {
        sleep_ms(0).await;
        leptos::task::tick().await;
    }

    /// The sentence and both answers, in the footer's order: Cancel first, the verb last.
    /// The verb is a plain button — nothing here is a form, so nothing submits.
    #[wasm_bindgen_test]
    fn the_sentence_and_both_answers_are_drawn() {
        let el = confirm(RwSignal::new(true), RwSignal::new(0), 0, Ok(()));
        element_saying(&el, SENTENCE);
        assert_eq!(
            footer_labels(&el),
            vec!["Cancel".to_string(), "Remove".to_string()],
            "Cancel first, the verb last"
        );
        let remove = button(&el, "Remove");
        assert_eq!(remove.get_attribute("type").as_deref(), Some("button"));
        assert!(
            !remove.has_attribute("form"),
            "no form to be associated with"
        );
        assert!(
            remove.class_name().contains("danger"),
            "the verb carries the Danger variant: {}",
            remove.class_name()
        );
        assert!(
            !button(&el, "Cancel").class_name().contains("danger"),
            "and Cancel does not"
        );
    }

    /// Cancel chooses nothing: the dialog closes and the action is never asked.
    #[wasm_bindgen_test]
    async fn cancel_closes_it_without_running_the_action() {
        let open = RwSignal::new(true);
        let ran = RwSignal::new(0);
        let el = confirm(open, ran, 0, Ok(()));
        button(&el, "Cancel").click();
        settle().await;
        assert!(!open.get_untracked(), "closed");
        assert_eq!(ran.get_untracked(), 0, "and nothing ran");
    }

    /// `Ok` closes it. The caller's reload happens inside the action, before it returns.
    #[wasm_bindgen_test]
    async fn the_verb_runs_the_action_and_closes_on_ok() {
        let open = RwSignal::new(true);
        let ran = RwSignal::new(0);
        let el = confirm(open, ran, 0, Ok(()));
        button(&el, "Remove").click();
        settle().await;
        assert_eq!(ran.get_untracked(), 1, "the click reached the action");
        assert!(!open.get_untracked(), "and success closed it");
    }

    /// The reason lands inside the dialog element, not in the page behind it, and the
    /// dialog stays open with both answers live again.
    #[wasm_bindgen_test]
    async fn a_refusal_is_drawn_inside_the_dialog_and_leaves_it_open() {
        let open = RwSignal::new(true);
        let el = confirm(
            open,
            RwSignal::new(0),
            0,
            Err("No permission to remove quilt-example."),
        );
        button(&el, "Remove").click();
        settle().await;

        assert!(
            open.get_untracked(),
            "a refused confirmation stays on screen"
        );
        let alert = dialog(&el)
            .query_selector("[role=alert]")
            .unwrap()
            .expect("the reason, announced, inside the dialog");
        assert!(
            alert
                .text_content()
                .unwrap_or_default()
                .contains("No permission"),
            "it says what the backend said: {:?}",
            alert.text_content()
        );
        assert!(!button(&el, "Remove").disabled(), "the verb is live again");
        assert!(!button(&el, "Cancel").disabled(), "and so is Cancel");
    }

    /// Both answers, not just the verb: Cancel would not stop a write already in flight,
    /// and a dialog that closed on it would say it had. A second press of the verb is
    /// dropped, not queued.
    #[wasm_bindgen_test]
    async fn both_answers_are_refused_while_the_action_runs() {
        let open = RwSignal::new(true);
        let ran = RwSignal::new(0);
        let el = confirm(open, ran, 50, Ok(()));
        button(&el, "Remove").click();
        settle().await;

        let remove = button(&el, "Remove");
        let cancel = button(&el, "Cancel");
        assert!(remove.disabled(), "the verb, so it cannot fire twice");
        assert_eq!(remove.get_attribute("aria-busy").as_deref(), Some("true"));
        assert!(cancel.disabled(), "and Cancel, which would not stop it");
        assert!(open.get_untracked(), "still open while it runs");

        remove.click();
        settle().await;
        assert_eq!(ran.get_untracked(), 1, "one command, not two");

        sleep_ms(80).await;
        leptos::task::tick().await;
        assert!(
            !open.get_untracked(),
            "and it closes once the action settles"
        );
    }

    /// The dialog unmounted mid-run — its caller rebuilt it over the same `open` —
    /// takes its session with it. The settling action must not read the disposed
    /// session, and its success still closes the caller's flag.
    #[wasm_bindgen_test]
    async fn a_success_after_the_dialog_unmounted_still_closes_the_caller_s_flag() {
        let open = RwSignal::new(true);
        let shown = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <Show when=move || shown.get()>
                    <ConfirmDialog
                        open=open
                        title="Remove package"
                        consequence=SENTENCE
                        confirm=Submit::new("Remove", || async {
                            sleep_ms(50).await;
                            Ok(())
                        })
                    />
                </Show>
            }
        });
        button(&el, "Remove").click();
        settle().await;
        shown.set(false);
        leptos::task::tick().await;

        sleep_ms(80).await;
        leptos::task::tick().await;
        assert!(!open.get_untracked(), "closed by the settled success");
    }

    /// Focus is asked for Cancel and for nothing else. The attribute is asserted rather
    /// than `document.activeElement`: no test in this crate reads focus, and headless
    /// Firefox under the runner gives no guarantee that a window without OS focus runs the
    /// focusing steps visibly — while `autofocus` is the contract `showModal()` honours.
    /// Order is asserted too, so a UA that focuses the first control lands on Cancel.
    #[wasm_bindgen_test]
    fn focus_is_asked_for_cancel_and_nothing_else() {
        let el = confirm(RwSignal::new(true), RwSignal::new(0), 0, Ok(()));
        assert!(button(&el, "Cancel").has_attribute("autofocus"));
        assert!(!button(&el, "Remove").has_attribute("autofocus"));
        assert_eq!(
            dialog(&el)
                .query_selector_all("[autofocus]")
                .unwrap()
                .length(),
            1,
            "one element asks, or the platform picks"
        );
        assert_eq!(
            footer_labels(&el)[0],
            "Cancel",
            "and it is the first control"
        );
    }

    /// Escape is `Dialog`'s own `cancel`, which closes — Cancel's answer. While the action
    /// runs the dialog refuses it, because dismissing neither stops the write nor makes its
    /// result irrelevant.
    #[wasm_bindgen_test]
    async fn escape_answers_cancel_except_while_the_action_runs() {
        let open = RwSignal::new(true);
        let el = confirm(open, RwSignal::new(0), 50, Ok(()));
        let dialog = dialog(&el);

        // `cancel` is cancelable, so `defaultPrevented` is the whole question: the UA
        // closes unless something prevented it.
        let escape = || {
            let init = web_sys::EventInit::new();
            init.set_cancelable(true);
            let ev = web_sys::Event::new_with_event_init_dict("cancel", &init).unwrap();
            dialog.dispatch_event(&ev).unwrap();
            ev.default_prevented()
        };

        assert!(
            !escape(),
            "Escape answers Cancel while nothing is in flight"
        );

        button(&el, "Remove").click();
        settle().await;
        assert!(escape(), "and is held while the command runs");

        sleep_ms(80).await;
        leptos::task::tick().await;
        assert!(!escape(), "and answers again once it settles");
    }
}
