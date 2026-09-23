use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::components::buttons;
use crate::components::layout;
use crate::components::{Layout, Notification};

/// Handle a command error that left the surface with nothing to render.
///
/// - `session_absent` → `/login`, carrying the host and a `back` target
/// - `setup_required` → `/setup`
/// - anything else → inline error page (keeps the URL, so reload retries)
///
/// Navigating is this function's policy, not the error's instruction: every
/// caller here has already failed to load. A surface with a local fallback
/// handles the state there and never arrives. Kinds a sign-in cannot fix —
/// `registry_url_missing` — are left unmatched on purpose.
pub fn handle_or_display(error: &str, notification: RwSignal<Option<Notification>>) -> AnyView {
    if let Ok(parsed) = serde_json::from_str::<ErrorResponse>(error) {
        match parsed.kind.as_str() {
            "session_absent" => {
                let host = parsed.host.filter(|h| !h.is_empty());
                match host {
                    Some(host) => {
                        let navigate = use_navigate();
                        let back = current_path_and_query();
                        let back_encoded = urlencoding::encode(&back);
                        navigate(
                            &format!("/login?host={host}&back={back_encoded}"),
                            NavigateOptions::default(),
                        );
                        ().into_any()
                    }
                    None => render_page_error(&parsed.message, notification),
                }
            }
            "setup_required" => {
                let navigate = use_navigate();
                navigate("/setup", NavigateOptions::default());
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
    let body = view! {
        <div class="qui-page-error container">
            <h1 class="title">"Error"</h1>
            <p class="message">{message}</p>
            <div class="button-group">
                <buttons::ReloadPage on_click=on_reload />
                <buttons::OpenDotQuilt on_click=on_dot_quilt />
                <buttons::GoHome />
            </div>
        </div>
    };
    // A caller that failed inside its own shell already drew the appbar.
    if layout::inside_layout() {
        body.into_any()
    } else {
        view! { <Layout breadcrumbs=vec![] notification=notification>{body}</Layout> }.into_any()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use leptos_router::components::Router;
    use wasm_bindgen_test::*;

    fn appbars(el: &web_sys::Element) -> u32 {
        el.query_selector_all(".layout-appbar").unwrap().length()
    }

    #[wasm_bindgen_test]
    fn a_page_that_failed_before_its_shell_gets_one_from_the_error_page() {
        let el = mount(|| {
            view! {
                <Router>{handle_or_display("boom", RwSignal::new(None))}</Router>
            }
        });
        assert_eq!(appbars(&el), 1, "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    fn a_page_that_failed_inside_its_shell_keeps_one_appbar() {
        let el = mount(|| {
            let notification = RwSignal::new(None);
            view! {
                <Router>
                    <Layout breadcrumbs=vec![] notification=notification>
                        {handle_or_display("boom", notification)}
                    </Layout>
                </Router>
            }
        });
        assert!(
            el.text_content().unwrap_or_default().contains("boom"),
            "markup was {}",
            el.inner_html()
        );
        assert_eq!(appbars(&el), 1, "markup was {}", el.inner_html());
    }
}
