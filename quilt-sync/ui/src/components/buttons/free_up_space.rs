use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn FreeUpSpace(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] busy: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) disabled=busy small=true link=true>
            {move || if busy.get().unwrap_or(false) { "Freeing up space\u{2026}" } else { "Free up space" }}
        </IconButton>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{button_saying, mount};
    use wasm_bindgen_test::*;

    /// While a sweep runs the button says so and cannot start a second one.
    #[wasm_bindgen_test]
    fn is_disabled_and_says_so_while_busy() {
        let el = mount(|| view! { <FreeUpSpace on_click=|_| {} busy=true /> });
        assert!(button_saying(&el, "Freeing up space\u{2026}").disabled());
    }

    #[wasm_bindgen_test]
    fn is_enabled_when_idle() {
        let el = mount(|| view! { <FreeUpSpace on_click=|_| {} /> });
        assert!(!button_saying(&el, "Free up space").disabled());
    }
}
