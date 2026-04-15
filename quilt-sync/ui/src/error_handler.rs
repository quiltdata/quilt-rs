use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::components::buttons;
use crate::components::{Layout, Notification};

/// Handle a command error by either navigating to an error/login/setup page
/// or rendering an inline error page.
///
/// - `login_required` → navigates to `/login`
/// - `setup_required` → navigates to `/setup`
/// - anything else → renders an error page inline (preserves the original URL
///   so a browser reload retries the failed page)
pub fn handle_or_display(error: &str, notification: RwSignal<Option<Notification>>) -> AnyView {
    if let Ok(parsed) = serde_json::from_str::<ErrorResponse>(error) {
        match parsed.kind.as_str() {
            "login_required" => {
                let host = parsed.host.filter(|h| !h.is_empty());
                match host {
                    Some(host) => {
                        let navigate = use_navigate();
                        let back = current_path_and_query();
                        let back_encoded = urlencoding::encode(&back);
                        navigate(
                            &format!("/login?host={host}&back={back_encoded}"),
                            Default::default(),
                        );
                        ().into_any()
                    }
                    None => render_page_error(&parsed.message, notification),
                }
            }
            "setup_required" => {
                let navigate = use_navigate();
                navigate("/setup", Default::default());
                ().into_any()
            }
            _ => render_page_error(&parsed.message, notification),
        }
    } else {
        render_page_error(error, notification)
    }
}

fn render_page_error(message: &str, notification: RwSignal<Option<Notification>>) -> AnyView {
    let message = message.to_string();
    let on_reload = move |_| {
        let _ = web_sys::window().and_then(|w| w.location().reload().ok());
    };
    let on_dot_quilt = move |_| {
        leptos::task::spawn_local(async move {
            let _ = commands::debug_dot_quilt().await;
        });
    };
    view! {
        <Layout breadcrumbs=vec![] notification=notification>
            <div class="qui-page-error container">
                <h1 class="title">"Error"</h1>
                <p class="message">{message}</p>
                <div class="button-group">
                    <buttons::ReloadPage on_click=on_reload />
                    <buttons::OpenDotQuilt on_click=on_dot_quilt />
                    <buttons::GoHome />
                </div>
            </div>
        </Layout>
    }
    .into_any()
}

/// Get the current browser path and query string (e.g. "/installed-package?namespace=user/pkg").
fn current_path_and_query() -> String {
    web_sys::window()
        .and_then(|w| {
            let loc = w.location();
            let path = loc.pathname().ok()?;
            let search = loc.search().ok().unwrap_or_default();
            Some(format!("{path}{search}"))
        })
        .unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct ErrorResponse {
    kind: String,
    message: String,
    #[serde(default)]
    host: Option<String>,
}
