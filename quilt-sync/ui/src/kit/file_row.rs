//! One recently-changed file.
//!
//! The row opens the file; the package tag navigates to its package. Both are
//! primary enough to be one click, which is why the tag is a real link and the row
//! is not an anchor.
//!
//! Up to three secondary actions live in the row and are always visible, not
//! revealed on hover. They are not on the packages view's rows, and the asymmetry
//! is deliberate: a file is a thing you act on, a package is a place you go. Two of
//! the three are optional — see `on_open_catalog` and `on_copy_uri` — because a
//! button with nothing behind it is chrome.

use leptos::ev::KeyboardEvent;
use leptos::ev::MouseEvent;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::IconButton;
use super::IconButtonVariant;
use super::RelativeTime;
use super::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/kit/file_row.module.scss");

// Inline rather than shared: these three are the only icons in the kit so far, and
// a real icon set is its own component. The box-arrow specifically means "leaves
// the application" — a plain arrow would be ambiguous next to the package tag,
// which is also navigation.
fn folder_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.3" stroke-linejoin="round">
            <path d="M1.75 4.25h3.9l1.4 1.9h7.2v7.6H1.75z" />
        </svg>
    }
    .into_any()
}

fn catalog_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.3" stroke-linejoin="round">
            <path d="M12.25 9.5v4.25H2.25V3.75H6.5" />
            <path d="M9.5 2.25h4.25V6.5M13.75 2.25 8 8" />
        </svg>
    }
    .into_any()
}

fn copy_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.3">
            <rect x="5.6" y="5.6" width="8.1" height="8.1" rx="1.4" />
            <path d="M10.4 3.3V2.9c0-.33-.27-.6-.6-.6H2.9c-.33 0-.6.27-.6.6v6.9c0 .33.27.6.6.6h.4" />
        </svg>
    }
    .into_any()
}

/// True only when the row's own root has focus.
///
/// The package link and the action buttons bubble their key presses through the
/// root's handler, and each already activates itself — a row that also opened the
/// file on their Enter would do two things at once.
fn targets_the_row(ev: &KeyboardEvent) -> bool {
    match (ev.target(), ev.current_target()) {
        (Some(target), Some(row)) => js_sys::Object::is(target.as_ref(), row.as_ref()),
        _ => false,
    }
}

/// Dispatch the row's own click, so the keyboard and the mouse cannot drift.
fn activate(ev: &KeyboardEvent) {
    if let Some(row) = ev.current_target()
        && let Ok(row) = row.dyn_into::<web_sys::HtmlElement>()
    {
        row.click();
    }
}

#[component]
pub fn FileRow(
    /// Logical key, shown whole and truncated from the left when it will not fit.
    #[prop(into)]
    path: String,
    #[prop(into)] package: String,
    #[prop(into)] package_href: String,
    at: EpochMillis,
    /// Opens the file in whatever application the OS associates with it. Bound to
    /// the row, not to a button — a button that duplicated the row's own affordance
    /// would be chrome.
    on_open: impl Fn(MouseEvent) + 'static,
    on_reveal: impl Fn(MouseEvent) + 'static,
    /// Optional: no command opens a *file* in the catalog yet. A button with
    /// nothing behind it is chrome — see `on_open`'s note. Pass it and the button
    /// appears.
    #[prop(optional, into)]
    on_open_catalog: Option<Callback<MouseEvent>>,
    /// Optional: the app has no clipboard access yet (`qhq-8mgw.13`).
    #[prop(optional, into)]
    on_copy_uri: Option<Callback<MouseEvent>>,
) -> impl IntoView {
    view! {
        // `role="button"` and `tabindex="0"` announce this row as a button and put
        // it in the tab order, so it owes what a native button gives for free.
        // Enter activates on the way down; Space waits for the release, because a
        // held key repeats its keydown and each repeat would open the file again.
        <div
            class=style::root
            role="button"
            tabindex="0"
            on:click=on_open
            on:keydown=move |ev: KeyboardEvent| {
                if !targets_the_row(&ev) {
                    return;
                }
                match ev.key().as_str() {
                    "Enter" => {
                        ev.prevent_default();
                        activate(&ev);
                    }
                    // Held, not yet activated — but the page must not scroll.
                    " " => ev.prevent_default(),
                    _ => {}
                }
            }
            on:keyup=move |ev: KeyboardEvent| {
                if targets_the_row(&ev) && ev.key() == " " {
                    ev.prevent_default();
                    activate(&ev);
                }
            }
        >
            <span class=style::path>{path}</span>
            // Stops propagation, or going to the package would also open the file.
            <a
                class=style::tag
                href=package_href
                on:click=|ev: MouseEvent| ev.stop_propagation()
            >
                {package}
            </a>
            <span class=style::time>
                <RelativeTime at=at />
            </span>
            <span class=style::actions on:click=|ev: MouseEvent| ev.stop_propagation()>
                <IconButton
                    icon=folder_icon()
                    aria_label="Reveal in directory"
                    variant=IconButtonVariant::Invisible
                    on_click=on_reveal
                />
                {on_open_catalog.map(|cb| {
                    view! {
                        <IconButton
                            icon=catalog_icon()
                            aria_label="Open in catalog"
                            variant=IconButtonVariant::Invisible
                            on_click=move |ev| cb.run(ev)
                        />
                    }
                })}
                {on_copy_uri.map(|cb| {
                    view! {
                        <IconButton
                            icon=copy_icon()
                            aria_label="Copy Quilt+S3 URI"
                            variant=IconButtonVariant::Invisible
                            on_click=move |ev| cb.run(ev)
                        />
                    }
                })}
            </span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// `main_page.rs`'s pattern: mount a view into a fresh, attached `div` and
    /// hand back the element to query against. No `Router` needed here — unlike
    /// `queue.rs`'s `mount`, nothing in this row navigates through one.
    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// The action span's buttons, in document order.
    fn buttons(el: &web_sys::Element) -> Vec<web_sys::Element> {
        let list = el.query_selector_all("button").unwrap();
        (0..list.length())
            .map(|i| list.get(i).unwrap().dyn_into().unwrap())
            .collect()
    }

    /// `queue.rs`'s pattern: `dyn_into` to the concrete element, then the DOM's
    /// own `.click()` — a real click, not a synthesized event.
    fn click(el: &web_sys::Element) {
        let el: web_sys::HtmlElement = el.clone().dyn_into().unwrap();
        el.click();
    }

    #[wasm_bindgen_test]
    fn a_row_given_no_catalog_or_copy_action_draws_neither_button() {
        // R1: two of this component's four actions have nothing behind them in the
        // app. A button that does nothing is chrome, which this file's own `on_open`
        // doc rejects in as many words.
        let el = mount(|| {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/installed-package?namespace=user/alpha"
                    at=0.0
                    on_open=|_| {}
                    on_reveal=|_| {}
                />
            }
        });
        let labels: Vec<String> = buttons(&el)
            .into_iter()
            .filter_map(|b| b.get_attribute("aria-label"))
            .collect();
        assert_eq!(
            labels,
            vec!["Reveal in directory".to_string()],
            "only the action with a command behind it"
        );
    }

    #[wasm_bindgen_test]
    fn a_row_given_every_action_draws_every_button() {
        // The optionality must not have deleted the capability: a later plan lights
        // these up by passing the props, and this is what pins that path.
        let el = mount(|| {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/installed-package?namespace=user/alpha"
                    at=0.0
                    on_open=|_| {}
                    on_reveal=|_| {}
                    on_open_catalog=Callback::new(|_| {})
                    on_copy_uri=Callback::new(|_| {})
                />
            }
        });
        let labels: Vec<String> = buttons(&el)
            .into_iter()
            .filter_map(|b| b.get_attribute("aria-label"))
            .collect();
        assert_eq!(
            labels,
            vec![
                "Reveal in directory".to_string(),
                "Open in catalog".to_string(),
                "Copy Quilt+S3 URI".to_string(),
            ],
            "order is the row's, not the caller's"
        );
    }

    /// A key press on the focused row, the way a keyboard user activates it.
    /// `bubbles` matters: the handler is on the root and the event must reach it.
    fn press(el: &web_sys::Element, event: &str, key: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_bubbles(true);
        let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(event, &init).unwrap();
        el.query_selector("[role=button]")
            .unwrap()
            .expect("the row root")
            .dispatch_event(&ev)
            .unwrap();
    }

    #[wasm_bindgen_test]
    fn enter_on_the_focused_row_opens_the_file() {
        // `role="button"` and `tabindex="0"` put this row in the tab order and
        // announce it as a button, so Enter has to do what a button does. A native
        // `<button>` gets that free; a div does not, and taking the role without
        // the key handling is worse than not being focusable at all — the row
        // invites a press and then ignores it.
        let opened = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/x"
                    at=0.0
                    on_open=move |_| opened.update(|n| *n += 1)
                    on_reveal=move |_| {}
                />
            }
        });

        press(&el, "keydown", "Enter");

        assert_eq!(opened.get_untracked(), 1, "Enter opens the file");
    }

    #[wasm_bindgen_test]
    fn space_opens_on_release_and_not_while_held() {
        // A native button activates on Enter's keydown but on Space's keyUP, and
        // the difference is not pedantry: activating on Space's keydown fires
        // again on every auto-repeat, so holding the key would open the file a
        // dozen times.
        let opened = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/x"
                    at=0.0
                    on_open=move |_| opened.update(|n| *n += 1)
                    on_reveal=move |_| {}
                />
            }
        });

        press(&el, "keydown", " ");
        assert_eq!(opened.get_untracked(), 0, "still held down, nothing yet");

        press(&el, "keyup", " ");
        assert_eq!(opened.get_untracked(), 1, "released, now it opens");
    }

    #[wasm_bindgen_test]
    fn enter_on_the_package_link_does_not_also_open_the_file() {
        // The link and the buttons bubble their key presses through the root's
        // handler. Each already activates itself, so a row that also opened the
        // file would do two things at once — navigate away AND launch an editor.
        // The click path is guarded by `stop_propagation`; this is the same guard
        // for the key path, and nothing else asserts it.
        let opened = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/x"
                    at=0.0
                    on_open=move |_| opened.update(|n| *n += 1)
                    on_reveal=move |_| {}
                />
            }
        });

        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Enter");
        init.set_bubbles(true);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        el.query_selector("a")
            .unwrap()
            .expect("the package link")
            .dispatch_event(&ev)
            .unwrap();

        assert_eq!(opened.get_untracked(), 0, "the link's Enter is the link's");
    }

    #[wasm_bindgen_test]
    fn a_key_the_row_does_not_claim_is_left_alone() {
        // Paired with the two above: without this, a handler that fired on every
        // key would pass both of them and steal Tab out of the tab order.
        let opened = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/x"
                    at=0.0
                    on_open=move |_| opened.update(|n| *n += 1)
                    on_reveal=move |_| {}
                />
            }
        });

        press(&el, "keydown", "Tab");
        press(&el, "keyup", "Tab");
        press(&el, "keydown", "a");

        assert_eq!(opened.get_untracked(), 0, "only Enter and Space activate");
    }

    #[wasm_bindgen_test]
    async fn clicking_the_row_opens_and_clicking_reveal_does_not_also_open() {
        // The row itself is the open affordance (`role="button"` on the root), and
        // the actions span stops propagation. Deleting that `stop_propagation` would
        // make every Reveal also open the file, which no assertion currently catches.
        let opened = RwSignal::new(0);
        let revealed = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/x"
                    at=0.0
                    on_open=move |_| opened.update(|n| *n += 1)
                    on_reveal=move |_| revealed.update(|n| *n += 1)
                />
            }
        });

        click(&buttons(&el)[0]);
        leptos::task::tick().await;

        assert_eq!(revealed.get_untracked(), 1);
        assert_eq!(
            opened.get_untracked(),
            0,
            "reveal must not also open the file"
        );
    }
}
