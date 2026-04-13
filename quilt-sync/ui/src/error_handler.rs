use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::commands;
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
                let navigate = use_navigate();
                let host = parsed.host.unwrap_or_default();
                let back = current_path_and_query();
                let back_encoded = urlencoding::encode(&back);
                navigate(
                    &format!("/login?host={host}&back={back_encoded}"),
                    Default::default(),
                );
                return view! {}.into_any();
            }
            "setup_required" => {
                let navigate = use_navigate();
                navigate("/setup", Default::default());
                return view! {}.into_any();
            }
            _ => {
                return render_page_error(&parsed.message, notification);
            }
        }
    } else {
        return render_page_error(error, notification);
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
                    <button class="qui-button" type="button" on:click=on_reload>
                        <span>"Reload page"</span>
                    </button>
                    <button class="qui-button" type="button" on:click=on_dot_quilt>
                        <span>"Open .quilt directory"</span>
                    </button>
                    <a class="qui-button primary" href="/installed-packages-list">
                        <span>"Go home"</span>
                    </a>
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
