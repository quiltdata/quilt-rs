use leptos::prelude::*;

#[component]
pub fn Spinner() -> impl IntoView {
    view! {
        <div class="q-spinner">
            <div></div>
            <div></div>
            <div></div>
            <div></div>
        </div>
    }
}
