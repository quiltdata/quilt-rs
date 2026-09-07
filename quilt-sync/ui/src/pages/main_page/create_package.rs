//! Create a package, as a modal over the main page.
//!
//! v2's own, not v1's `CreatePackagePopup`: the kit's `Dialog` traps focus and
//! handles Escape, neither of which v1's four hand-rolled overlays did. The two
//! commands underneath are v1's, unchanged.

use leptos::prelude::*;

use crate::commands;
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
        }
        was_open.set_value(now);
    });

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
                    // qhq-8mgw.47: this page has nowhere to show the backend's
                    // words, so the dialog stays open — which is itself the
                    // statement that nothing was created — and the Create
                    // button re-enables so the user can retry or cancel.
                    web_sys::console::error_1(&format!("package_create failed: {err}").into());
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
        </Dialog>
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
        // failure path. The dialog staying open IS the statement that nothing was
        // created, since this page has nowhere to print the backend's words.
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
}
