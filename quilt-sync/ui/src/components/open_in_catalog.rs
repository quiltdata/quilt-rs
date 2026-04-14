use leptos::prelude::*;

use crate::commands;

use super::IconButton;

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

    view! {
        <IconButton icon="/assets/img/icons/open_in_browser.svg" on_click=on_click small=small disabled=disabled>
            "Open in Catalog"
        </IconButton>
    }
}
