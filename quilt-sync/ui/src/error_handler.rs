use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::components::buttons;
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
///
/// The error page brings its own `Layout`: for a caller that failed before
/// drawing one. A caller already inside its `Layout` uses
/// [`handle_or_display_in_shell`].
pub fn handle_or_display(error: &str, notification: RwSignal<Option<Notification>>) -> AnyView {
    handle(error, notification, false)
}

/// [`handle_or_display`] for a caller that already drew its `Layout`, so the
/// error page does not draw a second one.
pub fn handle_or_display_in_shell(
    error: &str,
    notification: RwSignal<Option<Notification>>,
) -> AnyView {
    handle(error, notification, true)
}

fn handle(error: &str, notification: RwSignal<Option<Notification>>, in_shell: bool) -> AnyView {
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
                    None => render_page_error(&parsed.message, notification, in_shell),
                }
            }
            "setup_required" => {
                let navigate = use_navigate();
                navigate("/setup", NavigateOptions::default());
                ().into_any()
            }
            _ => render_page_error(&parsed.message, notification, in_shell),
        }
    } else {
        render_page_error(error, notification, in_shell)
    }
}

fn render_page_error(
    message: &str,
    notification: RwSignal<Option<Notification>>,
    in_shell: bool,
) -> AnyView {
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
    if in_shell {
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

/// The words a reader should see for a command's error text.
///
/// `Error::to_frontend_string` sends some errors as an [`ErrorResponse`] JSON —
/// `{"kind":…,"message":…}` — so a caller can route on `kind`. Drawn as it
/// arrives, that is the envelope, not the message. This returns the `message`
/// of such a JSON and any other text unchanged.
///
/// Apply it where the text is drawn, never where it is received: whatever routes
/// on `kind`, as [`handle_or_display`] does, needs the raw string.
#[must_use]
pub fn readable(error: &str) -> String {
    // An object first: serde would also read `ErrorResponse` from a two-item array.
    serde_json::from_str::<serde_json::Value>(error)
        .ok()
        .filter(serde_json::Value::is_object)
        .and_then(|value| serde_json::from_value::<ErrorResponse>(value).ok())
        .map_or_else(|| error.to_string(), |e| e.message)
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

    #[test]
    fn a_json_error_reads_as_its_message() {
        assert_eq!(
            readable(r#"{"kind":"access_denied","message":"No access.","host":"example.com"}"#),
            "No access."
        );
    }

    #[test]
    fn plain_text_reads_as_itself() {
        assert_eq!(readable("connection reset"), "connection reset");
        assert_eq!(readable(""), "");
    }

    #[test]
    fn json_of_another_shape_reads_as_itself() {
        for text in [
            r#"{"kind":"access_denied"}"#,
            r#"{"message":"No kind."}"#,
            r#"{"kind":"not_found","message":7}"#,
            r#"["access_denied","No access."]"#,
            r#""No access.""#,
        ] {
            assert_eq!(readable(text), text);
        }
    }

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
                        {handle_or_display_in_shell("boom", notification)}
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

    /// Merge, Commit and both package pages draw a `Layout` as their loading
    /// fallback and the error page beside it, not inside it.
    #[wasm_bindgen_test]
    async fn a_page_whose_fallback_is_a_shell_still_gets_one_on_failure() {
        let el = mount(|| {
            let notification = RwSignal::new(None);
            let data = LocalResource::new(|| async { Err::<(), _>("boom".to_string()) });
            view! {
                <Router>
                    <Suspense fallback=move || {
                        view! { <Layout breadcrumbs=vec![] notification=notification>"…"</Layout> }
                    }>
                        {move || Suspend::new(async move {
                            match data.await {
                                Ok(()) => ().into_any(),
                                Err(e) => handle_or_display(&e, notification),
                            }
                        })}
                    </Suspense>
                </Router>
            }
        });
        crate::test_support::sleep_ms(50).await;
        assert!(
            el.text_content().unwrap_or_default().contains("boom"),
            "markup was {}",
            el.inner_html()
        );
        assert_eq!(appbars(&el), 1, "markup was {}", el.inner_html());
    }
}
