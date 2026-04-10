use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::commands::{self, LoginErrorData};
use crate::components::{Layout, Spinner};

// ── Error page ──

#[component]
pub fn Error() -> impl IntoView {
    let notification = RwSignal::new(String::new());

    let query = use_query_map();
    let data = LocalResource::new(move || {
        let host = query.read().get("host").unwrap_or_default();
        let title = query.read().get("title");
        let error = query.read().get("error").unwrap_or_default();
        async move {
            commands::get_login_error_data(host, title, error).await
        }
    });

    view! {
        <Layout breadcrumbs=vec![] notification=notification>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => {
                            view! {
                                <ErrorContent data=d notification=notification />
                            }
                                .into_any()
                        }
                        Err(e) => {
                            let msg = format!("Failed to load error data: {e}");
                            view! {
                                <div class="qui-page-error container">
                                    <p>{msg}</p>
                                </div>
                            }
                                .into_any()
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}

// ── Main content (rendered after data loads) ──

#[component]
fn ErrorContent(data: LoginErrorData, notification: RwSignal<String>) -> impl IntoView {
    let on_reload = move |_| {
        let _ = web_sys::window().and_then(|w| w.location().reload().ok());
    };

    let on_dot_quilt = move |_| {
        leptos::task::spawn_local(async move {
            match commands::debug_dot_quilt().await {
                Ok(html) => notification.set(html),
                Err(e) => notification.set(format!("<div class=\"error\">{e}</div>")),
            }
        });
    };

    let login_host = data.login_host.clone();
    let back_encoded = urlencoding::encode("/installed-packages-list");
    let login_href = format!(
        "/login?host={}&back={back_encoded}",
        login_host
    );

    view! {
        <div class="qui-page-error container">
            <h1 class="title">{data.title}</h1>

            <p class="message" data-testid="error-msg">{data.message}</p>

            <div class="button-group">
                <button class="qui-button" type="button" on:click=on_reload>
                    <span>"Reload page"</span>
                </button>
                <button class="qui-button" type="button" on:click=on_dot_quilt>
                    <span>"Open .quilt directory"</span>
                </button>
                <a href=login_href>
                    <button class="qui-button" type="button">
                        <span>"Login"</span>
                    </button>
                </a>
                <a href="/installed-packages-list">
                    <button class="qui-button primary" type="button">
                        <span>"Go home"</span>
                    </button>
                </a>
            </div>
        </div>
    }
}
