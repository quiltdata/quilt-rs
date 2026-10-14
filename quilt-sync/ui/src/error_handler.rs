use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::components::buttons;
use crate::components::{Layout, Notification};
use crate::kit::error_text::ErrorResponse;
use crate::kit::readable;

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
    if let Some(target) = redirect(error) {
        let navigate = use_navigate();
        navigate(&target, NavigateOptions::default());
        return ().into_any();
    }
    render_page_error(&readable(error), notification, in_shell)
}

/// Where a load failure sends the reader instead of drawing itself, by the
/// rules [`handle_or_display`] lists: `/login` for a missing session with a
/// host, `/setup` for a missing data directory, and `None` for the rest.
///
/// For the v2 pages, which draw their own failure in their own frame but go
/// where v1 goes.
pub fn redirect(error: &str) -> Option<String> {
    let parsed = serde_json::from_str::<ErrorResponse>(error).ok()?;
    match parsed.kind.as_str() {
        "session_absent" => {
            let host = parsed.host.filter(|h| !h.is_empty())?;
            let back = current_path_and_query();
            let back_encoded = urlencoding::encode(&back);
            Some(format!("/login?host={host}&back={back_encoded}"))
        }
        "setup_required" => Some("/setup".to_string()),
        _ => None,
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
    fn sign_in_and_setup_are_the_only_redirects() {
        let login = redirect(r#"{"kind":"session_absent","message":"m","host":"quilt.test"}"#)
            .expect("a missing session with a host signs in");
        assert!(login.starts_with("/login?host=quilt.test&back="), "{login}");
        assert_eq!(
            redirect(r#"{"kind":"setup_required","message":"m"}"#).as_deref(),
            Some("/setup")
        );
        assert_eq!(redirect(r#"{"kind":"session_absent","message":"m"}"#), None);
        assert_eq!(
            redirect(r#"{"kind":"registry_url_missing","message":"m"}"#),
            None
        );
        assert_eq!(redirect("boom"), None);
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
