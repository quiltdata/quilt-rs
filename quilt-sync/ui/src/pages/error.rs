use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::commands::{self, LoginErrorData};
use crate::components::{Layout, Notification, Spinner};

// ── Error page ──

#[component]
pub fn Error() -> impl IntoView {
    let notification = RwSignal::new(None);
    let refetch = Trigger::new();

    let query = use_query_map();
    let data = LocalResource::new(move || {
        refetch.track();
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
                                <ErrorContent data=d notification=notification refetch=refetch />
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
fn ErrorContent(data: LoginErrorData, notification: RwSignal<Option<Notification>>, refetch: Trigger) -> impl IntoView {
    let on_reload = move |_| {
        refetch.notify();
    };

    let on_dot_quilt = move |_| {
        leptos::task::spawn_local(async move {
            match commands::debug_dot_quilt().await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
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
                <a class="qui-button" href=login_href>
                    <span>"Login"</span>
                </a>
                <a class="qui-button primary" href="/installed-packages-list">
                    <span>"Go home"</span>
                </a>
            </div>
        </div>
    }
}
