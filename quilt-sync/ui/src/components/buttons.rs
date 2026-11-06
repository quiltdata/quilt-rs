//! The two buttons of the app-wide popups: the quit prompt, the update bar and
//! the ignore popups.

use leptos::prelude::*;

/// The popup's answer.
#[component]
pub fn FormPrimary(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    #[prop(optional, into)] danger: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <button
            class="qui-button"
            // `danger` replaces `primary` rather than stacking on it, so the
            // error color never depends on CSS rule order to win.
            class:primary=move || !danger.get().unwrap_or(false)
            class:danger=move || danger.get().unwrap_or(false)
            type="button"
            prop:disabled=move || disabled.get().unwrap_or(false)
            on:click=on_click
        >
            <span>{children()}</span>
        </button>
    }
}

/// The way out of the popup; says "Cancel" unless given its own words.
#[component]
pub fn FormSecondary(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <button class="qui-button" type="button" on:click=on_click>
            <span>
                {match children {
                    Some(c) => c().into_any(),
                    None => "Cancel".into_any(),
                }}
            </span>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quilt_ui_kit::test_support::mount;
    use std::cell::Cell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn button(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        el.query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    #[wasm_bindgen_test]
    fn the_primary_is_primary_until_it_is_danger() {
        let el = mount(|| view! { <FormPrimary on_click=|_| {}>"Go"</FormPrimary> });
        let btn = button(&el);
        assert_eq!(btn.get_attribute("type").as_deref(), Some("button"));
        assert!(btn.class_list().contains("qui-button"));
        assert!(btn.class_list().contains("primary"));
        assert!(!btn.disabled());

        let el = mount(|| {
            view! { <FormPrimary on_click=|_| {} danger=true disabled=true>"Go"</FormPrimary> }
        });
        let btn = button(&el);
        assert!(btn.class_list().contains("danger"));
        assert!(!btn.class_list().contains("primary"));
        assert!(btn.disabled());
    }

    #[wasm_bindgen_test]
    fn the_secondary_says_cancel_and_runs_its_handler() {
        let clicked = Rc::new(Cell::new(false));
        let seen = clicked.clone();
        let el = mount(move || view! { <FormSecondary on_click=move |_| seen.set(true) /> });
        let btn = button(&el);
        assert_eq!(btn.text_content().unwrap(), "Cancel");
        assert!(!btn.class_list().contains("primary"));
        btn.click();
        assert!(clicked.get());
    }
}
