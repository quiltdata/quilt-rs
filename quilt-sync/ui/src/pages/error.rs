use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Layout, Spinner};
use crate::tauri;

// ── Data types (mirror the Tauri command response) ──

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginErrorData {
    pub title: String,
    pub message: String,
    pub login_host: String,
}

// ── Error page ──

#[component]
pub fn Error() -> impl IntoView {
    let notification = RwSignal::new(String::new());

    let data = LocalResource::new(move || async {
        let location = web_sys::window()
            .and_then(|w| w.location().href().ok())
            .unwrap_or_default();

        #[derive(Serialize)]
        struct Args {
            location: String,
        }
        tauri::invoke::<_, LoginErrorData>("get_login_error_data", &Args { location }).await
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
            match tauri::invoke_unit::<String>("debug_dot_quilt").await {
                Ok(html) => notification.set(html),
                Err(e) => notification.set(format!("<div class=\"error\">{e}</div>")),
            }
        });
    };

    let login_host = data.login_host.clone();
    let login_href = format!(
        "login.html#host={}&back=installed-packages-list.html",
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
                <a href="installed-packages-list.html">
                    <button class="qui-button primary" type="button">
                        <span>"Go home"</span>
                    </button>
                </a>
            </div>
        </div>
    }
}
