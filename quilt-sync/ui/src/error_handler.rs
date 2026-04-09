use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

/// Handle a command error by either navigating to an error/login/setup page
/// or setting the notification signal with the error message.
///
/// Call this from each page's `Suspense` error branch instead of showing
/// raw error text. It replicates the redirect logic from `load_page_command`.
pub fn handle_or_display(error: &str, notification: RwSignal<String>) -> AnyView {
    if let Ok(parsed) = serde_json::from_str::<ErrorResponse>(error) {
        match parsed.kind.as_str() {
            "login_required" => {
                let navigate = use_navigate();
                let host = parsed.host.unwrap_or_default();
                let back = parsed.back.unwrap_or_default();
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
                notification.set(format!(
                    r#"<div class="qui-notify error"><p>{}</p></div>"#,
                    parsed.message
                ));
            }
        }
    } else {
        notification.set(format!(
            r#"<div class="qui-notify error"><p>{error}</p></div>"#,
        ));
    }
    view! {}.into_any()
}

#[derive(serde::Deserialize)]
struct ErrorResponse {
    kind: String,
    message: String,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    back: Option<String>,
}
