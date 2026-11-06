use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use quilt_ui_kit::error_text::ErrorResponse;

/// Where a load failure sends the reader instead of drawing itself: `/login`
/// for a missing session with a host, `/setup` for a missing data directory,
/// and `None` for the rest. Kinds a sign-in cannot fix, such as
/// `registry_url_missing`, are left unmatched on purpose.
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

/// For a page read that failed on the page at path `on`: go where [`redirect`]
/// says, replacing this entry so Back does not land on a page that sends the
/// reader straight back, and say whether it went. The page draws nothing in
/// place then.
///
/// Never to `on` itself, so `/setup` and `/login` reading the error they
/// exist to fix draw their own failure instead of looping. `on` is the page's
/// path as it mounted, not the window's now: a page whose first read already
/// went has not left yet when its second fails.
pub fn follow_redirect(error: &str, on: &str) -> bool {
    let Some(target) = redirect(error) else {
        return false;
    };
    if target.split('?').next() == Some(on) {
        return false;
    }
    let navigate = use_navigate();
    navigate(
        &target,
        NavigateOptions {
            replace: true,
            ..NavigateOptions::default()
        },
    );
    true
}

/// The window's path, for [`follow_redirect`]'s `on`.
pub fn current_path() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default()
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
    use wasm_bindgen_test::*;

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
}
