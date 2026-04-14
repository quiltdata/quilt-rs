use leptos::prelude::*;

#[component]
pub fn IconButton(
    icon: &'static str,
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
    #[prop(optional)]
    primary: bool,
    #[prop(optional, into)]
    disabled: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    let class = match (small, primary) {
        (false, false) => "qui-button",
        (true, false) => "qui-button small",
        (false, true) => "qui-button primary",
        (true, true) => "qui-button primary small",
    };

    view! {
        <button
            class=class
            type="button"
            prop:disabled=move || disabled.get().unwrap_or(false)
            on:click=on_click
        >
            <img class="qui-icon" src=icon />
            {children()}
        </button>
    }
}
