use std::str::FromStr;

use rust_i18n::t;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use url::Url;

use crate::commands;
use crate::model;
use crate::oauth::OAuthState;
use crate::quilt;
use crate::routes;
use crate::telemetry::mixpanel::LoginFlow;
use crate::telemetry::prelude::*;
use crate::Error;
use crate::Result;

fn get_remote_package_url(current_url: &Url, uri_str: &str) -> Result<Url> {
    let uri: quilt::uri::S3PackageUri = uri_str.parse()?;
    Ok(routes::from_url(
        routes::Paths::RemotePackage(uri.clone()),
        current_url.to_owned(),
    ))
}

fn navigate_to_url(app_handle: &AppHandle, url: Url) -> Result {
    match app_handle.get_webview_window("main") {
        Some(win) => {
            win.navigate(url)?;
            win.set_focus()?;
            Ok(())
        }
        None => Err(Error::Window),
    }
}

/// Navigate to a Quilt package URI by parsing it and redirecting to the package page
fn navigate_to_package(app_handle: &AppHandle, uri_str: &str) -> Result {
    let win = app_handle.get_webview_window("main").ok_or(Error::Window)?;
    let current_url = win.url()?;
    let redirect_url = get_remote_package_url(&current_url, uri_str)?;
    navigate_to_url(app_handle, redirect_url)
}

/// Auth callback parameters parsed from a `quilt://` URL.
#[derive(Debug)]
struct AuthParams {
    code: String,
    state: String,
    host: quilt::uri::Host,
    redirect: Option<String>,
}

/// Parse auth callback query parameters from a `quilt://` URL.
fn parse_auth_params(url: &Url) -> Result<AuthParams> {
    // RFC 6749 §4.1.2.1: check for error response before looking for `code`.
    if let Some((_, error)) = url.query_pairs().find(|(k, _)| k == "error") {
        let description = url
            .query_pairs()
            .find(|(k, _)| k == "error_description")
            .map(|(_, v)| format!(": {v}"))
            .unwrap_or_default();
        return Err(Error::General(format!(
            "OAuth error — {error}{description}"
        )));
    }

    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| Error::General("Missing 'code' parameter in auth callback".into()))?;

    let host_str = url
        .query_pairs()
        .find(|(k, _)| k == "host")
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| Error::General("Missing 'host' parameter in auth callback".into()))?;

    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| Error::General("Missing 'state' parameter in auth callback".into()))?;

    let redirect = url
        .query_pairs()
        .find(|(k, _)| k == "redirect")
        .map(|(_, v)| v.into_owned());

    let host = quilt::uri::Host::from_str(&host_str)?;
    Ok(AuthParams {
        code,
        state,
        host,
        redirect,
    })
}

/// Handle `quilt://auth/callback?code=...&host=...&redirect=...` deep link
fn login_with_code(app_handle: &AppHandle, url: &Url) -> Result {
    debug!("login_with_code: parsing auth params from {}", url);
    let auth_params = parse_auth_params(url)?;
    let handle = app_handle.clone();
    let host = auth_params.host.clone();
    let host_str = host.to_string();
    let state = auth_params.state;
    let redirect_path: routes::Paths = auth_params
        .redirect
        .as_deref()
        .and_then(|r| {
            r.parse::<routes::Paths>()
                .map_err(|err| warn!("Failed to parse redirect '{}': {}", r, err))
                .ok()
        })
        .unwrap_or(routes::Paths::InstalledPackagesList);

    tauri::async_runtime::spawn(async move {
        let oauth_state = handle.state::<OAuthState>();
        let m = handle.state::<model::Model>();

        let result = match oauth_state
            .take_params(&host, auth_params.code.clone(), &state)
            .await
        {
            Ok((oauth_params, stored_location)) => {
                info!("OAuth 2.1 callback for host: {}", host_str);
                let login_result = model::login_oauth(&*m, &host, oauth_params).await;
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
                                )
                            })
                            .ok()
                    })
                    .unwrap_or(redirect_path);
                let telemetry = handle.state::<crate::telemetry::Telemetry>();
                telemetry
                    .track(crate::telemetry::MixpanelEvent::UserLoggedIn {
                        host: host_str.clone(),
                        flow: LoginFlow::OAuth,
                    })
                    .await;
                if let Err(err) = commands::navigate_after_login(&handle, final_path) {
                    error!("Failed to redirect after login: {}", err);
                    let error_path = routes::Paths::LoginError(
                        host.clone(),
                        t!("error.title").into(),
                        err.to_string(),
                    );
                    if let Some(win) = handle.get_webview_window("main") {
                        match win.url() {
                            Ok(win_url) => {
                                let error_url = routes::from_url(error_path, win_url);
                                if let Err(nav_err) = win.navigate(error_url) {
                                    error!(
                                        "Failed to navigate to error page after login: {}",
                                        nav_err
                                    );
                                }
                            }
                            Err(url_err) => {
                                error!(
                                    "Failed to get window URL when navigating to error page: {}",
                                    url_err
                                );
                            }
                        }
                    }
                }
            }
            (Err(err), _) => {
                error!("Failed to login via deep link: {}", err);
                let error_path = routes::Paths::LoginError(
                    host,
                    t!("login_error.title").into(),
                    err.to_string(),
                );
                if let Some(win) = handle.get_webview_window("main") {
                    match win.url() {
                        Ok(win_url) => {
                            let error_url = routes::from_url(error_path, win_url);
                            if let Err(nav_err) = win.navigate(error_url) {
                                error!("Failed to navigate to error page: {}", nav_err);
                            }
                        }
                        Err(url_err) => {
                            error!(
                                "Failed to get window URL when navigating to error page: {}",
                                url_err
                            );
                        }
                    }
                }
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

async fn wait_for_main_window(app_handle: &AppHandle) -> Result<()> {
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 10;
    const RETRY_DELAY_MS: u64 = 200;

    while attempts < MAX_ATTEMPTS {
        if let Some(window) = app_handle.get_webview_window("main") {
            // Check window is visible and URL is valid
            if let Ok(true) = window.is_visible() {
                if let Ok(url) = window.url() {
                    // Ensure the app has loaded (not about:blank)
                    if url.host().is_some() {
                        return Ok(());
                    }
                }
            }
        }

        attempts += 1;
        tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
    }

    Err(Error::Window)
}

fn handle_deep_link_navigation(app_handle: &AppHandle, urls: Vec<Url>) {
    let Some(first_url) = urls.first() else {
        return;
    };

    let url_str = first_url.to_string();
    let handle = app_handle.clone();

    tauri::async_runtime::spawn(async move {
        match wait_for_main_window(&handle).await {
            Ok(()) => {
                if let Err(err) = handle_deep_link_url(&handle, &url_str) {
                    error!("Failed to handle deep link '{}': {}", url_str, err);
                }
            }
            Err(_) => {
                error!(
                    "Failed to find ready main window, deep link lost: {}",
                    url_str
                );
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
        handle_deep_link_navigation(&handle_for_runtime, urls);
    });

    if let Ok(Some(urls)) = deep_link.get_current() {
        info!("Processing startup deep link: {:?}", urls);
        handle_deep_link_navigation(app_handle, urls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_remote_package_url() {
        let current_url = Url::parse("http://localhost:3000/").unwrap();
        let uri_str = "quilt+s3://bucket#package=foo/bar@hash";

        let result = get_remote_package_url(&current_url, uri_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_remote_package_url_invalid_uri() {
        let current_url = Url::parse("http://localhost:3000/").unwrap();
        let uri_str = "invalid-uri";

        let result = get_remote_package_url(&current_url, uri_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_auth_params() {
        let url =
            Url::parse("quilt://auth/callback?code=ABC123&host=test.quilt.dev&state=xyz").unwrap();
        let params = parse_auth_params(&url).unwrap();
        assert_eq!(params.code, "ABC123");
        assert_eq!(params.state, "xyz");
        assert_eq!(params.host.to_string(), "test.quilt.dev");
        assert_eq!(params.redirect, None);
    }

    #[test]
    fn test_parse_auth_params_with_redirect() {
        let url = Url::parse(
            "quilt://auth/callback?code=ABC123&host=test.quilt.dev&state=xyz&redirect=https%3A%2F%2Flocalhost%3A1234%2Fpages%2Fremote-package.html"
        ).unwrap();
        let params = parse_auth_params(&url).unwrap();
        assert_eq!(params.code, "ABC123");
        assert_eq!(params.state, "xyz");
        assert_eq!(params.host.to_string(), "test.quilt.dev");
        assert_eq!(
            params.redirect.as_deref(),
            Some("https://localhost:1234/pages/remote-package.html")
        );
    }

    #[test]
    fn test_parse_auth_params_missing_code() {
        let url = Url::parse("quilt://auth/callback?host=test.quilt.dev&state=xyz").unwrap();
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
        let url = Url::parse("quilt://auth/callback?code=ABC123&host=test.quilt.dev").unwrap();
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
}
