use leptos::prelude::*;

use crate::components::Layout;
use crate::components::buttons;

#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <Layout breadcrumbs=vec![] notification=RwSignal::new(None)>
            <div class="qui-page-error container">
                <h1 class="title">"Page not found"</h1>
                <p class="message">"The page you are looking for does not exist."</p>
                <div class="button-group">
                    <buttons::GoHome />
                </div>
            </div>
        </Layout>
    }
}
