use leptos::prelude::*;

#[component]
pub fn CommitLink(
    namespace: String,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    let href = format!("/commit?namespace={}", namespace);
    let class = if small { "qui-button small" } else { "qui-button" };

    view! {
        <a class=class href=href>
            <img class="qui-icon" src="/assets/img/icons/commit.svg" />
            <span>"Commit"</span>
        </a>
    }
}
