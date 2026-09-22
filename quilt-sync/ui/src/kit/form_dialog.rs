//! A [`Dialog`](super::Dialog) that holds a form and owns what happens when it is submitted.
//!
//! # `Dialog` arranges; this one acts
//!
//! `Dialog` is the shell — the element, the top layer, the focus trap, Escape, the title.
//! It takes a footer and draws it, and that is all it knows. Every caller that put a form
//! inside one then hand-wrote the same three things, and v1's four overlays got all three
//! wrong:
//!
//! 1. **Enter does nothing.** v1's popups are `div`s with click handlers. A user who types
//!    a bucket name and presses Return is ignored, which no other form on the machine does.
//! 2. **A second click re-fires the command.** Nothing disables the primary while the first
//!    one is in flight.
//! 3. **The rejection lands behind the dialog.** `set_remote` failing puts its message in
//!    the *page's* notification slot — underneath the modal that caused it. The user sees a
//!    form that did nothing.
//!
//! So this owns the submission: it awaits the caller's action, holds the in-flight state,
//! and shows what came back. What it deliberately does not own is `open`, which stays the
//! caller's in both directions exactly as `Dialog`'s does.
//!
//! # The primary lives outside the form it submits
//!
//! The fields are a `<form>` in the dialog's body; the buttons are in the footer, a
//! sibling. So the primary carries `form="<the form's id>"`, which is the platform's own
//! way to associate a submit button with a form it is not inside. That association is what
//! makes Return in a text field reach the primary — without it a form with two text fields
//! has no default button and implicit submission does nothing.
//!
//! # Success closes it; failure keeps it open with the reason
//!
//! The action returns `Result<(), String>`. `Ok` closes the dialog — the caller's own
//! reload or navigation happens inside the action, before it returns. `Err` leaves it open
//! and draws the message as a `Critical` [`Banner`](super::Banner) above the fields, which
//! is the one thing the user can act on and the one place they are looking.
//!
//! This is a **dialog-level** error and not
//! [`FormControl`](super::FormControl)'s. That one is per field and says which value is
//! wrong; this one is the backend declining the whole submission — no permission on the
//! bucket, a host that does not answer — and belongs to no field.
//!
//! # The form is sealed while the action runs
//!
//! Cancel is refused, because dismissing would not stop a write already in flight and a
//! dialog that closes on Cancel says it did. The **fields** are sealed for a nearer reason:
//! the values in flight are the values that were submitted, and a field left editable means
//! a refusal hands back a form mixing what was sent with what was typed after it. Both come
//! back the moment the action settles, so a refusal can be corrected.
//!
//! Escape is sealed too, through [`Dialog`]'s `held`. It was the one way out this
//! component could not refuse, and the hole it left was not merely a stale banner: a
//! rejection arriving after an Escape lands on a closed dialog, and reopening *before* the
//! action settled put the previous submission's reason on a form the reader had just
//! reopened. Holding Escape removes the state that made that reachable, and the clear on
//! open stays as the second line rather than the only one.
//!
//! # No submit makes it read-only
//!
//! `submit: None` draws no form and one `Close`. That is not a spare variant: a package
//! that has been pushed is pinned to its push history, so its remote can be *shown* and not
//! changed, and the release-notes popup has nothing to submit either. A second component
//! for "a dialog with one button" would differ from this one by nothing.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use leptos::prelude::*;

use super::Banner;
use super::BannerVariant;
use super::Button;
use super::ButtonVariant;
use super::Dialog;

stylance::import_crate_style!(style, "src/kit/form_dialog.module.scss");

/// The caller's action, boxed so the component's signature does not carry its future's
/// type. `Rc` and not `Arc`: this is a single-threaded wasm document, and the event handler
/// only needs to clone it.
type Action = Rc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>>>>>;

/// What a [`FormDialog`]'s primary is called and what it does.
///
/// The two travel together because neither is useful alone: a label with no action is a
/// button that lies, and an action with no label has nothing to draw.
#[derive(Clone)]
pub struct Submit {
    label: String,
    action: Action,
}

impl Submit {
    /// `label` is the verb on the primary — `Save`, `Create`, `Add to .quiltignore`.
    ///
    /// `action` runs on submit and its `Err` becomes the dialog's banner, so its message is
    /// read by a user rather than a log: it says what did not happen, not which layer
    /// refused.
    #[must_use]
    pub fn new<F, Fut>(label: impl Into<String>, action: F) -> Self
    where
        F: Fn() -> Fut + 'static,
        Fut: Future<Output = Result<(), String>> + 'static,
    {
        Self {
            label: label.into(),
            action: Rc::new(move || Box::pin(action())),
        }
    }
}

#[component]
pub fn FormDialog(
    /// The caller's, in both directions — see [`Dialog`]'s own `open`.
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    /// `None` makes this read-only: no form, and `Close` as the only way out.
    #[prop(optional)]
    submit: Option<Submit>,
    /// The fields. Wrapped in the `<form>` when there is something to submit.
    children: Children,
) -> impl IntoView {
    let submitting = RwSignal::new(false);
    // Derived once and used twice: `disabled` on Cancel and `loading` on the primary are
    // the same fact, and `MaybeProp` takes a signal rather than a bare closure.
    let busy = Signal::derive(move || submitting.get());
    let error = RwSignal::new(None::<String>);
    let form_id = super::unique_id("q-form");

    // Opening clears the last rejection — see the module doc on Escape.
    Effect::new(move |_| {
        if open.get() {
            error.set(None);
        }
    });

    let banner = view! {
        <Show when=move || error.get().is_some()>
            <Banner variant=BannerVariant::Critical on_dismiss=move |_| error.set(None)>
                {move || error.get().unwrap_or_default()}
            </Banner>
        </Show>
    };

    let (body, footer) = if let Some(Submit { label, action }) = submit {
        let on_submit = move |ev: leptos::ev::SubmitEvent| {
            // The default is a navigation, which in a webview is the app disappearing.
            ev.prevent_default();
            if submitting.get_untracked() {
                return;
            }
            submitting.set(true);
            error.set(None);
            let action = Rc::clone(&action);
            leptos::task::spawn_local(async move {
                let outcome = action().await;
                submitting.set(false);
                match outcome {
                    Ok(()) => open.set(false),
                    Err(message) => error.set(Some(message)),
                }
            });
        };
        let body = view! {
            {banner}
            <form id=form_id.clone() on:submit=on_submit>
                // `<fieldset disabled>` rather than a prop each field honours: it is the
                // platform's own way to seal a form, it reaches controls this component
                // has never heard of, and a caller cannot forget it.
                <fieldset class=style::fields disabled=busy>{children()}</fieldset>
            </form>
        }
        .into_any();
        let footer = view! {
            <Button
                disabled=busy
                on_click=move |_| open.set(false)
            >
                "Cancel"
            </Button>
            <Button
                variant=ButtonVariant::Primary
                submit=true
                form=form_id
                loading=busy
                on_click=move |_| ()
            >
                {label}
            </Button>
        }
        .into_any();
        (body, footer)
    } else {
        let body = view! { {banner} {children()} }.into_any();
        let footer = view! {
            <Button variant=ButtonVariant::Primary on_click=move |_| open.set(false)>
                "Close"
            </Button>
        }
        .into_any();
        (body, footer)
    };

    view! { <Dialog open=open title=title held=busy footer=footer>{body}</Dialog> }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::FormControl;
    use crate::kit::TextInput;
    use crate::test_support::element_saying;
    use crate::test_support::mount;
    use crate::test_support::sleep_ms;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// Two fields, so implicit submission has no default button of its own and the
    /// footer's `form` association is the only thing that could provide one.
    #[component]
    fn Fields() -> impl IntoView {
        let host = RwSignal::new("open.quiltdata.com".to_string());
        let bucket = RwSignal::new(String::new());
        view! {
            <FormControl
                label="Host"
                control=move |id| view! { <TextInput id=id value=host /> }.into_any()
            />
            <FormControl
                label="Bucket"
                control=move |id| view! { <TextInput id=id value=bucket /> }.into_any()
            />
        }
    }

    fn form(el: &web_sys::Element) -> web_sys::HtmlFormElement {
        el.query_selector("form")
            .unwrap()
            .expect("the form")
            .dyn_into()
            .unwrap()
    }

    fn buttons(el: &web_sys::Element) -> Vec<web_sys::HtmlButtonElement> {
        let all = el.query_selector_all("dialog button").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into())
            .collect()
    }

    fn primary(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        element_saying(el, label)
            .closest("button")
            .unwrap()
            .expect("the button holding those words")
            .dyn_into()
            .unwrap()
    }

    /// The association the module doc calls not cosmetic: without it a two-field form has
    /// no default button and Return does nothing.
    #[wasm_bindgen_test]
    fn the_primary_submits_the_form_it_is_not_inside() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new("Save", || async { Ok(()) })
                >
                    <Fields />
                </FormDialog>
            }
        });
        let save = primary(&el, "Save");
        assert_eq!(save.get_attribute("type").as_deref(), Some("submit"));
        assert_eq!(
            save.get_attribute("form"),
            form(&el).get_attribute("id"),
            "the primary names the form, and the form exists to be named"
        );
        assert!(
            save.closest("form").unwrap().is_none(),
            "and it is genuinely outside it — otherwise the attribute proves nothing"
        );
    }

    /// Clicking the primary reaches the action. This is the platform behaviour the `form`
    /// attribute buys, exercised rather than asserted.
    #[wasm_bindgen_test]
    async fn clicking_the_primary_runs_the_action() {
        let open = RwSignal::new(true);
        let ran = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new(
                        "Save",
                        move || async move {
                            ran.update(|n| *n += 1);
                            Ok(())
                        },
                    )
                >
                    <Fields />
                </FormDialog>
            }
        });
        primary(&el, "Save").click();
        sleep_ms(0).await;
        leptos::task::tick().await;
        assert_eq!(ran.get_untracked(), 1, "the click crossed into the form");
    }

    /// The rejection the page's notification slot used to swallow, now above the fields.
    #[wasm_bindgen_test]
    async fn a_rejection_is_drawn_in_the_dialog_and_leaves_it_open() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new(
                        "Save",
                        || async { Err("No permission to write to that bucket.".to_string()) },
                    )
                >
                    <Fields />
                </FormDialog>
            }
        });
        form(&el).request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;

        assert!(open.get_untracked(), "a form that failed stays on screen");
        let alert = el
            .query_selector("[role=alert]")
            .unwrap()
            .expect("the reason, announced");
        assert!(
            alert
                .text_content()
                .unwrap_or_default()
                .contains("No permission"),
            "it says what the backend said: {:?}",
            alert.text_content()
        );
    }

    /// `Ok` closes it. The caller's reload happens inside the action, before it returns.
    #[wasm_bindgen_test]
    async fn a_successful_submit_closes_it() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new("Save", || async { Ok(()) })
                >
                    <Fields />
                </FormDialog>
            }
        });
        form(&el).request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;
        assert!(!open.get_untracked());
    }

    /// Both buttons, not just the primary: Cancel would not stop a write already in
    /// flight, and a dialog that closed on it would say it had.
    #[wasm_bindgen_test]
    async fn both_buttons_are_refused_while_the_action_runs() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new(
                        "Save",
                        || async {
                            sleep_ms(50).await;
                            Ok(())
                        },
                    )
                >
                    <Fields />
                </FormDialog>
            }
        });
        form(&el).request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;

        let save = primary(&el, "Save");
        let cancel = primary(&el, "Cancel");
        assert!(save.disabled(), "the primary, so it cannot fire twice");
        assert_eq!(save.get_attribute("aria-busy").as_deref(), Some("true"));
        assert!(cancel.disabled(), "and Cancel, which would not stop it");
        assert!(open.get_untracked(), "still open while it runs");
    }

    /// The values in flight are the values that were submitted. Left editable, a refusal
    /// hands back a form mixing what was sent with what was typed after it.
    ///
    /// `:disabled` and not the input's own `disabled` property: the IDL attribute reflects
    /// only the element's own, and says nothing about a `<fieldset>` above it. The
    /// pseudo-class is what the browser actually acts on.
    #[wasm_bindgen_test]
    async fn the_fields_are_sealed_while_the_action_runs() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new(
                        "Save",
                        || async {
                            sleep_ms(50).await;
                            Ok(())
                        },
                    )
                >
                    <Fields />
                </FormDialog>
            }
        });
        let input = el.query_selector("input").unwrap().expect("a field");
        assert!(
            !input.matches(":disabled").unwrap(),
            "editable before anything is in flight"
        );

        form(&el).request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;
        assert!(
            input.matches(":disabled").unwrap(),
            "and sealed while the command runs"
        );

        sleep_ms(80).await;
        leptos::task::tick().await;
        assert!(
            !input.matches(":disabled").unwrap(),
            "and editable again once it settles, so a refusal can be corrected"
        );
    }

    /// A second submit while the first is in flight is dropped rather than queued.
    #[wasm_bindgen_test]
    async fn a_second_submit_mid_flight_is_dropped() {
        let open = RwSignal::new(true);
        let ran = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new(
                        "Save",
                        move || async move {
                            ran.update(|n| *n += 1);
                            sleep_ms(50).await;
                            Ok(())
                        },
                    )
                >
                    <Fields />
                </FormDialog>
            }
        });
        let the_form = form(&el);
        the_form.request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;
        the_form.request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;
        assert_eq!(ran.get_untracked(), 1, "one command, not two");
    }

    /// Escape closes the element while the action is still running, so a rejection can
    /// arrive after the close. Opening clears it rather than greeting the next reader.
    #[wasm_bindgen_test]
    async fn reopening_it_drops_the_last_rejection() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new("Save", || async { Err("Host did not answer.".to_string()) })
                >
                    <Fields />
                </FormDialog>
            }
        });
        form(&el).request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;
        assert!(el.query_selector("[role=alert]").unwrap().is_some());

        open.set(false);
        leptos::task::tick().await;
        open.set(true);
        leptos::task::tick().await;
        assert!(
            el.query_selector("[role=alert]").unwrap().is_none(),
            "a stale reason is not what the next reader asked to see"
        );
    }

    /// Escape is the element's own and fires `cancel` before it closes. While the action
    /// runs the dialog refuses it, because dismissing neither stops the write nor makes
    /// its result irrelevant — and a result landing on a closed dialog is what let a
    /// previous submission's reason greet someone who reopened it.
    #[wasm_bindgen_test]
    async fn escape_is_refused_while_the_action_runs() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog
                    open=open
                    title="Change bucket"
                    submit=Submit::new(
                        "Save",
                        || async {
                            sleep_ms(50).await;
                            Ok(())
                        },
                    )
                >
                    <Fields />
                </FormDialog>
            }
        });
        let dialog = el.query_selector("dialog").unwrap().expect("the dialog");

        // `cancel` is cancelable, so `defaultPrevented` is the whole question: the UA
        // closes unless something prevented it.
        let escape = || {
            let init = web_sys::EventInit::new();
            init.set_cancelable(true);
            let ev = web_sys::Event::new_with_event_init_dict("cancel", &init).unwrap();
            dialog.dispatch_event(&ev).unwrap();
            ev.default_prevented()
        };

        assert!(!escape(), "Escape closes it while nothing is in flight");

        form(&el).request_submit().unwrap();
        sleep_ms(0).await;
        leptos::task::tick().await;
        assert!(escape(), "and is refused while the command runs");

        sleep_ms(80).await;
        leptos::task::tick().await;
        assert!(!escape(), "and answers again once it settles");
    }

    /// The read-only shape: `Show remote` on a package pinned to its push history.
    #[wasm_bindgen_test]
    fn a_read_only_dialog_draws_no_form_and_one_way_out() {
        let open = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <FormDialog open=open title="Show remote">
                    <Fields />
                </FormDialog>
            }
        });
        assert!(
            el.query_selector("form").unwrap().is_none(),
            "nothing to submit, so no form to submit it"
        );
        let footer: Vec<String> = buttons(&el)
            .iter()
            .filter_map(|b| b.text_content())
            .filter(|t| !t.trim().is_empty())
            .collect();
        assert_eq!(footer, vec!["Close".to_string()], "one way out, {footer:?}");
    }
}
