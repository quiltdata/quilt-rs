use leptos::prelude::*;

#[component]
pub fn IconLink(
    href: String,
    icon: &'static str,
    #[prop(optional)]
    small: bool,
    #[prop(optional)]
    primary: bool,
    children: Children,
) -> impl IntoView {
    let class = match (small, primary) {
        (false, false) => "qui-button",
        (true, false) => "qui-button small",
        (false, true) => "qui-button primary",
        (true, true) => "qui-button primary small",
    };

    view! {
        <a class=class href=href>
            <img class="qui-icon" src=icon />
            <span>{children()}</span>
        </a>
    }
}
