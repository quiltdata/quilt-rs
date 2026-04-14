use leptos::prelude::*;

#[component]
pub fn MergeLink(
    namespace: String,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    let href = format!("/merge?namespace={}", namespace);
    let class = if small {
        "qui-button primary small"
    } else {
        "qui-button primary"
    };

    view! {
        <a class=class href=href>
            <img class="qui-icon" src="/assets/img/icons/merge.svg" />
            <span>"Merge"</span>
        </a>
    }
}
