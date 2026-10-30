use std::str::FromStr;
use std::sync::Arc;

use rust_i18n::t;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use url::Url;

use super::show_window;
use crate::engine::Error;
use crate::engine::Result;
use crate::engine::model;
use crate::engine::routes;
use crate::engine::services::Services;
use crate::engine::telemetry::MixpanelEvent;
use crate::engine::telemetry::event::{LoginEvent, LoginFlow};
use crate::engine::telemetry::prelude::*;

/// Send every window and tab to `path`.
fn navigate(services: &Services, path: routes::Paths) {
    let page = routes::page(path);
    debug!("Navigating every client to {}", page);
    services.events.emit(routes::NAVIGATE_EVENT, page);
}

/// The page of a Quilt package URI.
fn package_path(uri_str: &str) -> Result<routes::Paths> {
    Ok(routes::Paths::RemotePackage(uri_str.parse()?))
}

/// Show a Quilt package URI's page, in the window brought to the front.
fn navigate_to_package(app_handle: &AppHandle, uri_str: &str) -> Result {
    navigate(&app_handle.state::<Arc<Services>>(), package_path(uri_str)?);
    show_window(app_handle);
    Ok(())
}

/// Auth callback parameters parsed from a `quilt://` URL.
#[derive(Debug)]
struct AuthParams {
    code: String,
    state: String,
    host: quilt_uri::Host,
    redirect: Option<String>,
}

/// Parse auth callback query parameters from a `quilt://` URL.
fn parse_auth_params(url: &Url) -> Result<AuthParams> {
    // Collect into a HashMap keeping the first value for each key, matching
    // the original .find() semantics and guarding against HTTP Parameter Pollution.
    let mut params = std::collections::HashMap::new();
    for (k, v) in url.query_pairs() {
        params.entry(k).or_insert(v);
    }

    // RFC 6749 §4.1.2.1: check for error response before looking for `code`.
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .map(|v| format!(": {v}"))
            .unwrap_or_default();
        return Err(Error::General(format!(
            "OAuth error — {error}{description}"
        )));
    }

    let code = params
        .get("code")
        .map(std::string::ToString::to_string)
        .ok_or_else(|| Error::General("Missing 'code' parameter in auth callback".into()))?;

    let host_str = params
        .get("host")
        .map(std::string::ToString::to_string)
        .ok_or_else(|| Error::General("Missing 'host' parameter in auth callback".into()))?;

    let state = params
        .get("state")
        .map(std::string::ToString::to_string)
        .ok_or_else(|| Error::General("Missing 'state' parameter in auth callback".into()))?;

    let redirect = params.get("redirect").map(std::string::ToString::to_string);

    let host = quilt_uri::Host::from_str(&host_str)?;
    Ok(AuthParams {
        code,
        state,
        host,
        redirect,
    })
}

/// Where a deep-link login lands when the callback names a page, and where it
/// lands when it does not.
///
/// The default is `/`, which renders whichever main page the reader has switched
/// on. Naming a specific page here would land half the readers on the other one.
/// An unreadable redirect gets the same default rather than an error: the login
/// itself succeeded, and dropping the user nowhere is worse than dropping them
/// home.
fn landing_after_login(redirect: Option<&str>) -> routes::Paths {
    redirect
        .and_then(|r| {
            r.parse::<routes::Paths>()
                .map_err(|err| warn!("Failed to parse redirect '{}': {}", r, err))
                .ok()
        })
        .unwrap_or(routes::Paths::Home)
}

/// Handle `quilt://auth/callback?code=...&host=...&redirect=...` deep link
#[allow(
    clippy::too_many_lines,
    reason = "cohesive deep-link auth-callback handling"
)]
fn login_with_code(app_handle: &AppHandle, url: &Url) -> Result {
    debug!("login_with_code: parsing auth params from {}", url);
    let auth_params = parse_auth_params(url)?;
    let handle = app_handle.clone();
    let host = auth_params.host.clone();
    let host_str = host.to_string();
    let state = auth_params.state;
    let redirect_path = landing_after_login(auth_params.redirect.as_deref());

    tauri::async_runtime::spawn(async move {
        let s = handle.state::<Arc<Services>>();
        let oauth_state = &s.oauth;
        let m = &*s.model;

        let result = match oauth_state
            .take_params(&host, auth_params.code.clone(), &state)
            .await
        {
            Ok((oauth_params, stored_location)) => {
                info!("OAuth 2.1 callback for host: {}", host_str);
                let login_result = model::login_oauth(m, &host, oauth_params).await;
                (login_result, stored_location)
            }
            Err(err) => {
                error!(
                    "OAuth callback rejected for {}, aborting: {}",
                    host_str, err
                );
                (Err(err), None)
            }
        };

        match result {
            (Ok(()), stored_location) => {
                let final_path = stored_location
                    .as_deref()
                    .and_then(|loc| {
                        loc.parse::<routes::Paths>()
                            .map_err(|err| {
                                error!(
                                    "Login succeeded but stored redirect '{}' \
                                     is not a valid route: {}; \
                                     falling back to default page",
                                    loc, err
                                );
                            })
                            .ok()
                    })
                    .unwrap_or(redirect_path);
                s.telemetry.track(MixpanelEvent::UserLoggedIn(LoginEvent {
                    host,
                    flow: LoginFlow::OAuth,
                }));
                navigate(&s, final_path);
            }
            (Err(err), _) => {
                error!("Failed to login via deep link: {}", err);
                navigate(
                    &s,
                    routes::Paths::LoginError(
                        host,
                        t!("login_error.title").into(),
                        err.to_string(),
                    ),
                );
            }
        }
    });

    Ok(())
}

/// Dispatch a deep link URL to the appropriate handler based on scheme
pub fn handle_deep_link_url(app_handle: &AppHandle, url_str: &str) -> Result {
    debug!("handle_deep_link_url: {}", url_str);
    let url: Url = url_str.parse().map_err(Error::ParseUrl)?;

    match url.scheme() {
        "quilt+s3" => navigate_to_package(app_handle, url_str),
        "quilt" => login_with_code(app_handle, &url),
        scheme => {
            error!("Unknown deep link scheme: {}", scheme);
            Err(Error::General(format!(
                "Unknown deep link scheme: {scheme}"
            )))
        }
    }
}

/// Wait until a window or tab listens, so a deep link that arrives with the
/// launch isn't sent to nobody. The window loads the UI first, which takes a
/// moment.
async fn wait_for_a_client(app_handle: &AppHandle) -> Result<()> {
    const MAX_ATTEMPTS: u32 = 50;
    const RETRY_DELAY_MS: u64 = 200;

    let services = Arc::clone(&app_handle.state::<Arc<Services>>());
    for _ in 0..MAX_ATTEMPTS {
        if services.events.has_listeners() {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
    }
    Err(Error::General("No window or tab connected".into()))
}

fn handle_deep_link_navigation(app_handle: &AppHandle, urls: &[Url]) {
    let Some(first_url) = urls.first() else {
        return;
    };

    let url_str = first_url.to_string();
    let handle = app_handle.clone();

    tauri::async_runtime::spawn(async move {
        match wait_for_a_client(&handle).await {
            Ok(()) => {
                if let Err(err) = handle_deep_link_url(&handle, &url_str) {
                    error!("Failed to handle deep link '{}': {}", url_str, err);
                }
            }
            Err(_) => {
                error!("No window or tab connected, deep link lost: {}", url_str);
            }
        }
    });
}

pub fn setup_deep_link_handler(app_handle: &AppHandle) {
    let deep_link = app_handle.deep_link();

    let handle_for_runtime = app_handle.clone();
    deep_link.on_open_url(move |event| {
        let urls = event.urls();
        // On Linux, the single-instance plugin already handles deep links
        // via argv. Skip on_open_url to avoid duplicate handling.
        if cfg!(target_os = "linux") {
            debug!(
                "Skipping on_open_url (handled by single-instance): {:?}",
                urls
            );
            return;
        }
        info!("Processing runtime deep link: {:?}", urls);
        handle_deep_link_navigation(&handle_for_runtime, &urls);
    });

    if let Ok(Some(urls)) = deep_link.get_current() {
        info!("Processing startup deep link: {:?}", urls);
        handle_deep_link_navigation(app_handle, &urls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_link_opens_its_package_page() {
        let path = package_path("quilt+s3://bucket#package=foo/bar@hash").unwrap();
        assert!(
            routes::page(path).starts_with("/remote-package?"),
            "a remote package page"
        );
        assert!(package_path("invalid-uri").is_err());
    }

    #[tokio::test]
    async fn navigating_sends_every_client_the_page() {
        let data_dir = tempfile::tempdir().unwrap();
        let services = Services::init(data_dir.path().to_path_buf(), semver::Version::new(0, 0, 0))
            .await
            .unwrap();
        let mut events = services.events.subscribe();
        navigate(&services, routes::Paths::Settings);
        assert_eq!(
            events.recv().await.unwrap(),
            (
                routes::NAVIGATE_EVENT.to_owned(),
                serde_json::json!("/settings")
            )
        );
    }

    #[test]
    fn test_parse_auth_params() {
        let url =
            Url::parse("quilt://auth/callback?code=ABC123&host=quilt.test&state=xyz").unwrap();
        let params = parse_auth_params(&url).unwrap();
        assert_eq!(params.code, "ABC123");
        assert_eq!(params.state, "xyz");
        assert_eq!(params.host.to_string(), "quilt.test");
        assert_eq!(params.redirect, None);
    }

    #[test]
    fn test_parse_auth_params_with_redirect() {
        let url = Url::parse(
            "quilt://auth/callback?code=ABC123&host=quilt.test&state=xyz&redirect=https%3A%2F%2Flocalhost%3A1234%2Fremote-package"
        ).unwrap();
        let params = parse_auth_params(&url).unwrap();
        assert_eq!(params.code, "ABC123");
        assert_eq!(params.state, "xyz");
        assert_eq!(params.host.to_string(), "quilt.test");
        assert_eq!(
            params.redirect.as_deref(),
            Some("https://localhost:1234/remote-package")
        );
    }

    #[test]
    fn test_parse_auth_params_missing_code() {
        let url = Url::parse("quilt://auth/callback?host=quilt.test&state=xyz").unwrap();
        let result = parse_auth_params(&url);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_auth_params_missing_host() {
        let url = Url::parse("quilt://auth/callback?code=ABC123&state=xyz").unwrap();
        let result = parse_auth_params(&url);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_auth_params_missing_state() {
        let url = Url::parse("quilt://auth/callback?code=ABC123&host=quilt.test").unwrap();
        let result = parse_auth_params(&url);
        assert!(result.is_err());
    }

    // RFC 6749 §4.1.2.1: error response (e.g. user denied access) must surface
    // the OAuth error instead of a generic "Missing 'code' parameter" message.
    #[test]
    fn test_parse_auth_params_error_response() {
        let url = Url::parse(
            "quilt://auth/callback?error=access_denied&error_description=User+denied+access",
        )
        .unwrap();
        let err = parse_auth_params(&url).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("access_denied"),
            "expected error code in message: {msg}"
        );
        assert!(
            msg.contains("User denied access"),
            "expected description in message: {msg}"
        );
    }

    #[test]
    fn test_parse_auth_params_error_without_description() {
        let url = Url::parse("quilt://auth/callback?error=server_error").unwrap();
        let err = parse_auth_params(&url).unwrap_err();
        assert!(err.to_string().contains("server_error"));
    }

    #[test]
    fn a_callback_with_no_redirect_lands_home() {
        assert_eq!(landing_after_login(None), routes::Paths::Home);
    }

    #[test]
    fn a_callback_that_names_a_page_lands_there() {
        assert_eq!(
            landing_after_login(Some("/settings")),
            routes::Paths::Settings
        );
    }

    /// The addresses the package and commit pages send as `back`.
    #[test]
    fn a_login_from_a_package_or_commit_page_lands_there() {
        assert_eq!(
            landing_after_login(Some(
                "/installed-package?namespace=org%2Fpkg&filter=unmodified"
            )),
            routes::Paths::InstalledPackage(
                ("org", "pkg").into(),
                routes::EntriesFilter::for_installed_package()
            )
        );
        // A deep link's warning rides along in `back`; the page still parses.
        assert_eq!(
            landing_after_login(Some(
                "/installed-package?namespace=org%2Fpkg&filter=unmodified&mismatch=abc&mrbucket=b"
            )),
            routes::Paths::InstalledPackage(
                ("org", "pkg").into(),
                routes::EntriesFilter::for_installed_package()
            )
        );
        assert_eq!(
            landing_after_login(Some("/commit?namespace=org%2Fpkg")),
            routes::Paths::Commit(("org", "pkg").into(), routes::EntriesFilter::default())
        );
    }

    #[test]
    fn an_unreadable_redirect_lands_home_rather_than_nowhere() {
        // The login succeeded by this point, so there is no failure to report to
        // the user — only a choice of page.
        assert_eq!(
            landing_after_login(Some("/not-a-page")),
            routes::Paths::Home
        );
    }
}
