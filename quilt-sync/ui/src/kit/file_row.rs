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

use leptos::ev::MouseEvent;
use leptos::prelude::*;

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
        <div class=style::root role="button" tabindex="0" on:click=on_open>
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
