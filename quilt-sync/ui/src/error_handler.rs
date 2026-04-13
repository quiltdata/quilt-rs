use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::components::{Layout, Notification};

/// Handle a command error by either navigating to an error/login/setup page
/// or rendering a Layout with the error as a notification.
///
/// Call this from each page's `Suspense` error branch instead of showing
/// raw error text. It replicates the redirect logic from `load_page_command`.
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
                notification.set(Some(Notification::Error(parsed.message)));
            }
        }
    } else {
        notification.set(Some(Notification::Error(error.to_string())));
    }
    view! {
        <Layout breadcrumbs=vec![] notification=notification>
            <div></div>
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
