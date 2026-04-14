use leptos::prelude::*;

use crate::commands;

#[component]
pub fn OpenInCatalog(
    url: Option<String>,
    #[prop(optional)]
    small: bool,
    #[prop(optional)]
    disabled: bool,
) -> impl IntoView {
    let on_click = move |_| {
        if let Some(url) = url.clone() {
            leptos::task::spawn_local(async move {
                let _ = commands::open_in_web_browser(url).await;
            });
        }
    };

    let class = if small { "qui-button small" } else { "qui-button" };

    view! {
        <button class=class type="button" prop:disabled=disabled on:click=on_click>
            <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
            <span>"Open in Catalog"</span>
        </button>
    }
}
