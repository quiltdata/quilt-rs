//! The prompt that stands between a quit and an interrupted apply.
//!
//! The backend defers the quit and raises this; the answer decides. Drawn with
//! the app's own popup idiom rather than an OS dialog — see the desktop spec's
//! shared GUI chrome.

use leptos::prelude::*;

use crate::commands;
use crate::components::buttons;
use crate::tauri as tauri_bridge;

/// Event the backend raises when a quit would interrupt an apply. Kept in
/// lockstep with `tray.rs`.
const QUIT_PROMPT_EVENT: &str = "quit-prompt";

/// The question itself, free of the event plumbing so it can be rendered — and
/// tested — on its own.
#[component]
pub fn QuitPromptCard(
    on_quit: impl Fn(leptos::ev::MouseEvent) + 'static,
    on_stay: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        // No click-outside dismiss: this is a question, and the ways out are
        // the two buttons. A stray click on the backdrop should not decide
        // whether the sync survives.
        <div class="popup-overlay">
            <div class="popup-content">
                <div class="quit-prompt">
                    <h2 class="section-title">"Still syncing"</h2>
                    <p>
                        "QuiltSync is writing files right now. Quitting will interrupt it and "
                        "leave this package part-way between two revisions."
                    </p>
                    <div class="quit-prompt-actions">
                        <buttons::FormPrimary on_click=on_stay>"Stay"</buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_quit>
                            "Quit anyway"
                        </buttons::FormSecondary>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// App-level listener. Mounted once beside the other app-level surfaces, so a
/// quit is answerable from whatever screen is open.
#[component]
pub fn QuitPrompt() -> impl IntoView {
    let visible = RwSignal::new(false);

    let listener = tauri_bridge::listen::<()>(QUIT_PROMPT_EVENT, move |()| {
        visible.set(true);
        // Report the prompt on screen at once. Until this lands the backend
        // treats the quit as unanswerable and lets it through after a short
        // grace period — which is the intended behavior when the window
        // cannot ask, and must not be the behavior when it can.
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::quit_prompt_shown().await {
                leptos::logging::error!("quit prompt ack failed: {err}");
            }
        });
    });
    on_cleanup(move || drop(listener));

    let on_quit = move |_| {
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::quit_confirm().await {
                leptos::logging::error!("quit failed: {err}");
            }
        });
    };
    let on_stay = move |_| {
        visible.set(false);
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::quit_cancel().await {
                leptos::logging::error!("quit cancel failed: {err}");
            }
        });
    };

    move || {
        visible
            .get()
            .then(|| view! { <QuitPromptCard on_quit=on_quit on_stay=on_stay /> })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    fn button_saying(el: &web_sys::Element, text: &str) -> web_sys::HtmlElement {
        let buttons = el.query_selector_all("button").unwrap();
        for i in 0..buttons.length() {
            let node = buttons.item(i).unwrap();
            let button: web_sys::HtmlElement = node.dyn_into().unwrap();
            if button.text_content().unwrap_or_default().contains(text) {
                return button;
            }
        }
        panic!("no button saying {text:?}; markup was {}", el.inner_html());
    }

    // Both ways out have to be on screen. A prompt offering only "quit anyway"
    // would be a warning, not a question, and one offering only "stay" would
    // trap the user in an app that will not close.
    #[wasm_bindgen_test]
    fn both_choices_are_offered() {
        let el = mount(|| {
            view! { <QuitPromptCard on_quit=|_| {} on_stay=|_| {} /> }
        });
        let text = el.text_content().unwrap();
        assert!(text.contains("Quit anyway"), "markup was {text}");
        assert!(text.contains("Stay"), "markup was {text}");
    }

    #[wasm_bindgen_test]
    fn choosing_to_stay_does_not_also_signal_a_quit() {
        let quit = std::rc::Rc::new(std::cell::Cell::new(false));
        let stay = std::rc::Rc::new(std::cell::Cell::new(false));
        let (q, s) = (quit.clone(), stay.clone());
        let el = mount(move || {
            view! {
                <QuitPromptCard
                    on_quit=move |_| q.set(true)
                    on_stay=move |_| s.set(true)
                />
            }
        });

        button_saying(&el, "Stay").click();

        assert!(stay.get(), "staying must be reported");
        assert!(!quit.get(), "staying must not quit");
    }

    #[wasm_bindgen_test]
    fn choosing_to_quit_anyway_signals_a_quit() {
        let quit = std::rc::Rc::new(std::cell::Cell::new(false));
        let q = quit.clone();
        let el = mount(move || {
            view! { <QuitPromptCard on_quit=move |_| q.set(true) on_stay=|_| {} /> }
        });

        button_saying(&el, "Quit anyway").click();

        assert!(quit.get(), "quitting anyway must be reported");
    }
}
