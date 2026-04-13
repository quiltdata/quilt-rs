use leptos::prelude::*;

use crate::components::Layout;

#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <Layout breadcrumbs=vec![] notification=RwSignal::new(None)>
            <div class="qui-page-error container">
                <h1 class="title">"Page not found"</h1>
                <p class="message">"The page you are looking for does not exist."</p>
                <div class="button-group">
                    <a class="qui-button primary" href="/installed-packages-list">
                        <span>"Go home"</span>
                    </a>
                </div>
            </div>
        </Layout>
    }
}
