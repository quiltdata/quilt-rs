//! Create a package, as a modal over the main page.
//!
//! v2's own, not v1's `CreatePackagePopup`: the kit's `Dialog` traps focus and
//! handles Escape, neither of which v1's four hand-rolled overlays did. The two
//! commands underneath are v1's, unchanged.

use leptos::prelude::*;

use crate::commands;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Dialog;
use crate::kit::FormControl;
use crate::kit::TextInput;

stylance::import_crate_style!(style, "src/pages/main_page/create_package.module.scss");

#[component]
pub fn CreatePackageDialog(open: RwSignal<bool>, reload: Trigger) -> impl IntoView {
    let namespace = RwSignal::new(String::new());
    let source = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    // The backend's words about the last attempt, shown in the dialog rather than in
    // `PageLayout`'s banner slot: `Dialog` uses `show_modal()`, so anything in the page's
    // own flow sits behind the backdrop in a lower layer, dimmed and out of tab order.
    let failure = RwSignal::new(None::<String>);
    // Trimmed, as v1 does: a name of spaces is not a name, and the backend
    // would refuse it after a round trip.
    let can_create =
        Signal::derive(move || !namespace.get().trim().is_empty() && !submitting.get());

    // This dialog is mounted once, permanently, outside the page's `Transition`
    // (a refetch must not close it mid-typing) — unlike v1's `CreatePackagePopup`,
    // which sat inside a `Show` and so got a fresh instance, and a fresh form,
    // every open. Without this the previous package's name would sit in the
    // field on the next open, inviting an accidental duplicate submission.
    //
    // Resets only on the transition to `true`, tracked by `was_open` — never on
    // close, and never while already open. Closing on a failed create must keep
    // what the user typed, so they can retry or fix it; that only works because
    // this effect does not also fire when `open` goes false or stays put.
    let was_open = StoredValue::new(false);
    Effect::new(move |_| {
        let now = open.get();
        if now && !was_open.get_value() {
            namespace.set(String::new());
            source.set(String::new());
            submitting.set(false);
            failure.set(None);
        }
        was_open.set_value(now);
    });

    let on_create = move |_| {
        let ns = namespace.get_untracked().trim().to_string();
        if ns.is_empty() || submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        let src = source.get_untracked();
        let src = (!src.is_empty()).then_some(src);
        leptos::task::spawn_local(async move {
            match commands::package_create(ns, src, None).await {
                Ok(_) => {
                    // The new package arrives through the page's own read — the
                    // same trigger Refresh notifies — rather than a second
                    // fetch path (R7).
                    reload.notify();
                    open.set(false);
                }
                Err(err) => {
                    // The dialog stays open on a failure and closes only on a
                    // success or on the user's Cancel/Escape, so what they typed
                    // survives to be fixed and resubmitted.
                    failure.set(Some(err));
                    submitting.set(false);
                }
            }
        });
    };

    let on_cancel = move |_| open.set(false);

    view! {
        <Dialog
            open=open
            title="Create package"
            footer=view! {
                <Button on_click=on_cancel>"Cancel"</Button>
                <Button
                    variant=ButtonVariant::Primary
                    disabled=Signal::derive(move || !can_create.get())
                    loading=submitting
                    on_click=on_create
                >
                    "Create"
                </Button>
            }
                .into_any()
        >
            // Above the fields, not beside one: `package_create` returns an opaque
            // `String`, so nothing here can say which field is at fault, and
            // `FormControl`'s `error` would set `aria-invalid` on a guess.
            {move || {
                failure
                    .get()
                    .map(|message| {
                        view! {
                            <Banner
                                variant=BannerVariant::Critical
                                on_dismiss=move |_| failure.set(None)
                            >
                                {message}
                            </Banner>
                        }
                    })
            }}
            <PackageFields namespace=namespace source=source />
        </Dialog>
    }
}

/// What the form asks for, and the picker that fills one of the two.
///
/// Split from the dialog around them: these own the fields, while the dialog owns
/// what happens on submit and what it says when that fails.
#[component]
fn PackageFields(namespace: RwSignal<String>, source: RwSignal<String>) -> impl IntoView {
    // v1's directory picker, unchanged: the chosen path becomes the (disabled)
    // field's text. A cancelled picker returns `Err`, and the field is left as
    // it was.
    let on_browse = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(path) = commands::open_directory_picker().await {
                source.set(path);
            }
        });
    };

    view! {
        <FormControl
            label="Package name"
            caption="Two parts, separated by a slash — user/plate-07."
            required=true
            control=move |id| {
                view! {
                    <TextInput
                        id=id
                        value=namespace
                        placeholder="owner/package-name"
                        autofocus=true
                    />
                }
                    .into_any()
            }
        />
        // A path the user picks with the OS dialog rather than types, so the
        // text field is disabled and the Browse button is the control.
        <FormControl
            label="Folder to add"
            caption="Optional. You can add files later."
            control=move |id| {
                view! {
                    <div class=style::folder_row>
                        <TextInput
                            id=id
                            value=source
                            placeholder="No folder chosen"
                            disabled=true
                        />
                        <Button on_click=on_browse>"Browse…"</Button>
                    </div>
                }
                    .into_any()
            }
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// `kit/file_row.rs`'s pattern: mount a view into a fresh, attached `div` and
    /// hand back the element to query against.
    fn mount_dialog(open: RwSignal<bool>) -> web_sys::Element {
        let reload = Trigger::new();
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), move || {
            view! { <CreatePackageDialog open=open reload=reload /> }
        })
        .forget();
        container.into()
    }

    /// Reads the `<dialog>` element's `open` PROPERTY, not an attribute —
    /// `kit::Dialog` calls `show_modal()`/`close()` on the element directly and
    /// keeps the caller's signal in step via `on:close`, so there is no `open`
    /// attribute to read.
    fn is_open(el: &web_sys::Element) -> bool {
        el.query_selector("dialog")
            .unwrap()
            .expect("the dialog element")
            .dyn_into::<web_sys::HtmlDialogElement>()
            .unwrap()
            .open()
    }

    /// The namespace field, found by its placeholder — the only `TextInput` in
    /// this dialog that is not disabled.
    fn namespace_field(el: &web_sys::Element) -> web_sys::HtmlInputElement {
        el.query_selector("input[placeholder='owner/package-name']")
            .unwrap()
            .expect("the package name field")
            .dyn_into()
            .unwrap()
    }

    /// Every `<button>` in document order.
    fn buttons(el: &web_sys::Element) -> Vec<web_sys::HtmlButtonElement> {
        let list = el.query_selector_all("button").unwrap();
        (0..list.length())
            .map(|i| list.get(i).unwrap().dyn_into().unwrap())
            .collect()
    }

    /// A footer button, found by its text — never positionally, since a footer
    /// reorder must not silently repoint these.
    fn button_labelled(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        buttons(el)
            .into_iter()
            .find(|b| b.text_content().unwrap_or_default().trim() == label)
            .unwrap_or_else(|| panic!("a button labelled `{label}`"))
    }

    fn create_button(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        button_labelled(el, "Create")
    }

    fn cancel_button(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        button_labelled(el, "Cancel")
    }

    /// `queue.rs`'s pattern: `dyn_into` to the concrete element, then the DOM's
    /// own `.click()` — a real click, not a synthesized event.
    fn click(el: &web_sys::HtmlButtonElement) {
        el.click();
    }

    /// Types into a field the way a user does — set the property, then fire the
    /// event `TextInput` listens for (`kit/text_input.rs:64`: `on:input`).
    fn type_into(field: &web_sys::HtmlInputElement, text: &str) {
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    /// `main_page.rs`'s own pattern for waiting out a `spawn_local` that this
    /// test cannot otherwise observe finishing.
    async fn sleep_ms(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    #[wasm_bindgen_test]
    fn a_closed_dialog_is_not_showing() {
        // The dialog is mounted in the document permanently (see
        // `CreatePackageDialog`'s own doc) — this asserts the element's `open`
        // property, not its presence in the DOM.
        let open = RwSignal::new(false);
        let el = mount_dialog(open);

        assert!(!is_open(&el), "a closed dialog is not showing");
    }

    #[wasm_bindgen_test]
    async fn create_is_refused_while_the_namespace_is_empty() {
        // v1 returns early on an empty namespace (`installed_packages_list.rs:848`).
        // A create with no name is not a thing the backend can do, and finding that
        // out through a rejected round trip would be worse than not offering it.
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;

        assert!(create_button(&el).disabled(), "no name, nothing to create");

        type_into(&namespace_field(&el), "user/new-package");
        leptos::task::tick().await;

        assert!(
            !create_button(&el).disabled(),
            "a named package can be created"
        );
    }

    #[wasm_bindgen_test]
    async fn a_namespace_of_only_spaces_is_still_empty() {
        // v1 trims before checking (`installed_packages_list.rs:847`). Without the
        // trim the button enables on a space and the backend refuses it.
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;

        type_into(&namespace_field(&el), "   ");
        leptos::task::tick().await;

        assert!(create_button(&el).disabled(), "whitespace is not a name");
    }

    #[wasm_bindgen_test]
    async fn cancel_closes_the_dialog_and_creates_nothing() {
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;
        type_into(&namespace_field(&el), "user/new-package");

        click(&cancel_button(&el));
        leptos::task::tick().await;

        assert!(
            !open.get_untracked(),
            "the signal the caller owns is written back"
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_create_leaves_the_dialog_open_and_the_button_usable() {
        // There is no Tauri host here, so the invoke rejects — which is exactly the
        // failure path. The dialog stays open because the operation did not succeed
        // and the user did not dismiss it; what it says is asserted separately, in
        // `a_failed_create_says_why_inside_the_dialog`.
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;
        type_into(&namespace_field(&el), "user/new-package");
        leptos::task::tick().await;

        click(&create_button(&el));
        sleep_ms(50).await;

        assert!(open.get_untracked(), "still open after a failure");
        assert!(
            !create_button(&el).disabled(),
            "and usable again, or the user is stuck with a dialog that does nothing"
        );
        // The open-transition reset must not have fired here: `open` never went
        // false and back to true during this failure, so what the user typed is
        // still theirs to fix or resubmit.
        assert_eq!(
            namespace_field(&el).value(),
            "user/new-package",
            "a failed create must not discard what the user typed"
        );
    }

    #[wasm_bindgen_test]
    async fn reopening_the_dialog_clears_what_was_typed_before() {
        // This dialog is mounted once, permanently (unlike v1's `CreatePackagePopup`,
        // which sat inside a `Show` and so got a fresh instance every open). Without
        // a reset on the open transition, the previous package's name would still be
        // sitting in the field, inviting an accidental duplicate-namespace submit.
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;
        type_into(&namespace_field(&el), "user/first-package");
        leptos::task::tick().await;

        click(&cancel_button(&el));
        leptos::task::tick().await;
        open.set(true);
        // Two ticks: the reset runs inside the effect that reacts to `open`
        // itself changing, so the write to `namespace` it makes is a second
        // wave the DOM binding needs one more flush to pick up.
        leptos::task::tick().await;
        leptos::task::tick().await;

        assert_eq!(
            namespace_field(&el).value(),
            "",
            "a fresh open must not carry over the last package's name"
        );
    }

    /// The failure message, which only exists once a create has failed.
    fn failure(el: &web_sys::Element) -> Option<web_sys::Element> {
        el.query_selector("dialog [role='alert']").unwrap()
    }

    /// The banner's own close control, found by its label — its face is an SVG,
    /// so `button_labelled` cannot see it.
    fn dismiss_failure(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        el.query_selector("dialog button[aria-label='Dismiss']")
            .unwrap()
            .expect("the failure's dismiss control")
            .dyn_into()
            .unwrap()
    }

    #[wasm_bindgen_test]
    async fn a_failed_create_says_why_inside_the_dialog() {
        // qhq-8mgw.47. The message goes in the dialog and not in `PageLayout`'s
        // banner slot because `Dialog` uses `show_modal()`: the page sits behind
        // a 45% backdrop in a lower layer, where a notice is dimmed and its
        // dismiss unreachable by pointer or by tab.
        //
        // The same failure is asked for directly first, so this asserts that the
        // BACKEND'S OWN WORDS arrive — not merely that some message appeared.
        // There is no Tauri host here, so both calls reject with the same message.
        //
        // Its first line only: with no host the rejection is a thrown `TypeError`
        // whose text carries a JS stack, and the stack names the frame that called,
        // so the two differ past the message itself. A real host returns a plain
        // string and the whole of it lands in the banner.
        let err = commands::package_create("user/new-package".to_string(), None, None)
            .await
            .expect_err("no Tauri host, so the invoke rejects");
        let expected = err.lines().next().expect("a message to compare");

        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;
        type_into(&namespace_field(&el), "user/new-package");
        leptos::task::tick().await;

        click(&create_button(&el));
        sleep_ms(50).await;

        let shown = failure(&el)
            .expect("a failed create must say why, and inside the dialog")
            .text_content()
            .unwrap_or_default();
        assert!(
            shown.contains(expected),
            "the message must carry the backend's words; got {shown:?}"
        );
    }

    #[wasm_bindgen_test]
    async fn dismissing_the_failure_clears_it_and_nothing_else() {
        // The banner's dismiss is about the message, not the dialog: the dialog
        // closes on a success or on the user's Cancel/Escape, never on its own.
        // Clearing the message must also leave the typed name to resubmit.
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;
        type_into(&namespace_field(&el), "user/new-package");
        leptos::task::tick().await;
        click(&create_button(&el));
        sleep_ms(50).await;
        assert!(failure(&el).is_some(), "precondition: the failure is shown");

        click(&dismiss_failure(&el));
        leptos::task::tick().await;

        assert!(failure(&el).is_none(), "dismissing clears the message");
        assert!(
            open.get_untracked(),
            "and only the message — the dialog is the user's to close"
        );
        assert_eq!(
            namespace_field(&el).value(),
            "user/new-package",
            "dismissing a message must not discard what the user typed"
        );
    }

    #[wasm_bindgen_test]
    async fn reopening_the_dialog_clears_a_previous_failure() {
        // The same open-transition reset that clears the fields, for the same
        // reason: this dialog is mounted once, so without it the last attempt's
        // error would greet the next one, describing a create that is not this one.
        let open = RwSignal::new(true);
        let el = mount_dialog(open);
        leptos::task::tick().await;
        type_into(&namespace_field(&el), "user/new-package");
        leptos::task::tick().await;
        click(&create_button(&el));
        sleep_ms(50).await;
        assert!(failure(&el).is_some(), "precondition: the failure is shown");

        click(&cancel_button(&el));
        leptos::task::tick().await;
        open.set(true);
        // Two ticks, as `reopening_the_dialog_clears_what_was_typed_before`: the
        // reset writes from inside the effect that reacts to `open`.
        leptos::task::tick().await;
        leptos::task::tick().await;

        assert!(
            failure(&el).is_none(),
            "a fresh open must not carry the last attempt's failure"
        );
    }
}
