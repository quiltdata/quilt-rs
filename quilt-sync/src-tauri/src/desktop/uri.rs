use std::str::FromStr;
use std::sync::Arc;

use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use url::Url;

use super::show_window;
use crate::engine::Error;
use crate::engine::Result;
use crate::engine::commands::ACCOUNTS_CHANGED_EVENT;
use crate::engine::model;
use crate::engine::navigation;
use crate::engine::routes;
use crate::engine::services::Services;
use crate::engine::telemetry::MixpanelEvent;
use crate::engine::telemetry::event::{LoginEvent, LoginFlow};
use crate::engine::telemetry::prelude::*;

/// Send page `to` (a page id; `None` is the window) to `path`.
fn navigate(services: &Services, to: Option<&str>, path: routes::Paths) {
    let to = to.unwrap_or(navigation::WINDOW);
    debug!("Sending page {} to {}", to, path);
    services.navigations.send(&services.events, to, path);
}

/// The page of a Quilt package URI.
fn package_path(uri_str: &str) -> Result<routes::Paths> {
    Ok(routes::Paths::RemotePackage(uri_str.parse()?))
}

/// The engine's `link` (`{url}/?t={token}`) turned to the page of a Quilt
/// package URI. The page takes its token from `t` on any path.
fn package_link(link: &str, uri_str: &str) -> Result<String> {
    let url: Url = link.parse().map_err(Error::ParseUrl)?;
    let token = url
        .query_pairs()
        .find(|(key, _)| key == "t")
        .map(|(_, token)| token.into_owned())
        .unwrap_or_default();
    let mut url = routes::from_url(package_path(uri_str)?, url);
    url.query_pairs_mut().append_pair("t", &token);
    Ok(url.into())
}

/// The page to open on another process's engine: the package's, when this
/// launch came with a package link, else `link`'s home page. A login link
/// has nothing to finish there, since headless logs in by code.
fn launch_page(link: String, launched: Option<&Url>) -> String {
    let Some(uri) = launched.filter(|url| url.scheme() == "quilt+s3") else {
        return link;
    };
    package_link(&link, uri.as_str()).unwrap_or_else(|err| {
        eprintln!("Can't open {uri} in QuiltSync: {err}");
        link
    })
}

/// [`launch_page`] for the link this launch came with.
pub fn launch_link(app_handle: &AppHandle, link: String) -> String {
    let launched = app_handle.deep_link().get_current().ok().flatten();
    launch_page(link, launched.as_ref().and_then(|urls| urls.first()))
}

/// Show a Quilt package URI's page, in the window brought to the front.
fn navigate_to_package(app_handle: &AppHandle, uri_str: &str) -> Result {
    navigate(
        &app_handle.state::<Arc<Services>>(),
        None,
        package_path(uri_str)?,
    );
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
    debug!("login_with_code: parsing auth params from {}", for_log(url));
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

        let (landing, params) = oauth_state
            .take_params(&host, auth_params.code.clone(), &state)
            .await;
        let result = match params {
            Ok(oauth_params) => {
                info!("OAuth 2.1 callback for host: {}", host_str);
                model::login_oauth(m, &host, oauth_params).await
            }
            Err(err) => {
                error!(
                    "OAuth callback rejected for {}, aborting: {}",
                    host_str, err
                );
                Err(err)
            }
        };

        match result {
            Ok(()) => {
                let final_path = landing
                    .back
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
                s.watcher.action_needed().session_works(Some(&host));
                s.telemetry.track(MixpanelEvent::UserLoggedIn(LoginEvent {
                    host,
                    flow: LoginFlow::OAuth,
                }));
                s.events.emit(ACCOUNTS_CHANGED_EVENT, ());
                navigate(&s, landing.page.as_deref(), final_path);
            }
            Err(err) => {
                error!("Failed to login via deep link: {}", err);
                navigate(
                    &s,
                    landing.page.as_deref(),
                    routes::Paths::LoginError(host, "Login failed".into(), err.to_string()),
                );
            }
        }
    });

    Ok(())
}

/// Render a deep-link URL for a log line. An auth callback (`quilt://`) carries a
/// one-time authorization code in its query; dropping the query keeps that code
/// out of the log files (and so out of the support archive and the crash
/// reporter). A package link (`quilt+s3://`) holds nothing secret, so it is
/// logged whole.
fn for_log(url: &Url) -> String {
    if url.scheme() == "quilt" {
        let mut redacted = url.clone();
        redacted.set_query(None);
        redacted.to_string()
    } else {
        url.to_string()
    }
}

/// [`for_log`] for a deep-link string that has not been parsed yet. An
/// unparseable string still has anything after `?` dropped, in case it holds a
/// code.
pub(crate) fn for_log_str(raw: &str) -> String {
    match raw.parse::<Url>() {
        Ok(url) => for_log(&url),
        Err(_) => raw.split('?').next().unwrap_or_default().to_string(),
    }
}

/// Dispatch a deep link URL to the appropriate handler based on scheme
pub fn handle_deep_link_url(app_handle: &AppHandle, url_str: &str) -> Result {
    let url: Url = url_str.parse().map_err(Error::ParseUrl)?;
    debug!("handle_deep_link_url: {}", for_log(&url));

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

fn handle_deep_link_navigation(app_handle: &AppHandle, urls: &[Url]) {
    let Some(first_url) = urls.first() else {
        return;
    };

    // The window takes its page whenever it connects, so a link that arrives
    // with the launch needs no wait for it.
    let url_str = first_url.to_string();
    if let Err(err) = handle_deep_link_url(app_handle, &url_str) {
        error!(
            "Failed to handle deep link '{}': {}",
            for_log(first_url),
            err
        );
    }
}

pub fn setup_deep_link_handler(app_handle: &AppHandle) {
    let deep_link = app_handle.deep_link();

    let handle_for_runtime = app_handle.clone();
    deep_link.on_open_url(move |event| {
        let urls = event.urls();
        let logged: Vec<String> = urls.iter().map(for_log).collect();
        // On Linux, the single-instance plugin already handles deep links
        // via argv. Skip on_open_url to avoid duplicate handling.
        if cfg!(target_os = "linux") {
            debug!(
                "Skipping on_open_url (handled by single-instance): {:?}",
                logged
            );
            return;
        }
        info!("Processing runtime deep link: {:?}", logged);
        handle_deep_link_navigation(&handle_for_runtime, &urls);
    });

    if let Ok(Some(urls)) = deep_link.get_current() {
        let logged: Vec<String> = urls.iter().map(for_log).collect();
        info!("Processing startup deep link: {:?}", logged);
        handle_deep_link_navigation(app_handle, &urls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_auth_callback_is_logged_without_its_query() {
        let url: Url = "quilt://auth/callback?host=quilt.test&code=one-time-code&state=xyz"
            .parse()
            .unwrap();
        let logged = for_log(&url);
        assert!(
            !logged.contains("one-time-code"),
            "code is dropped: {logged}"
        );
        assert!(!logged.contains("code="), "query is dropped: {logged}");
        assert!(
            logged.contains("quilt://auth/callback"),
            "scheme, host and path are kept: {logged}"
        );
    }

    #[test]
    fn a_package_link_is_logged_whole() {
        let url: Url =
            "quilt+s3://bucket#package=foo/bar@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .parse()
                .unwrap();
        assert_eq!(for_log(&url), url.to_string());
    }

    #[test]
    fn an_unparseable_deep_link_is_logged_without_its_query() {
        let logged = for_log_str("not a url?code=one-time-code");
        assert!(
            !logged.contains("one-time-code"),
            "code is dropped: {logged}"
        );
    }

    #[test]
    fn a_package_link_opens_its_package_page() {
        let path = package_path("quilt+s3://bucket#package=foo/bar@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
        assert!(
            routes::page(path).starts_with("/remote-package?"),
            "a remote package page"
        );
        assert!(package_path("invalid-uri").is_err());
    }

    #[test]
    fn a_package_link_keeps_the_engine_and_its_token() {
        let uri = "quilt+s3://bucket#package=foo/bar@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let link = package_link("http://127.0.0.1:7878/?t=abc", uri).unwrap();
        let page = routes::page(package_path(uri).unwrap());
        assert_eq!(link, format!("http://127.0.0.1:7878{page}&t=abc"));
    }

    const LINK: &str = "http://127.0.0.1:7878/?t=abc";

    #[test]
    fn a_package_launch_opens_its_page_with_the_token() {
        let package: Url = "quilt+s3://bucket#package=foo/bar@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".parse().unwrap();
        assert_eq!(
            launch_page(LINK.into(), Some(&package)),
            package_link(LINK, package.as_str()).unwrap()
        );
    }

    #[test]
    fn a_launch_without_a_link_opens_the_home_page() {
        assert_eq!(launch_page(LINK.into(), None), LINK);
    }

    #[test]
    fn a_login_launch_opens_the_home_page() {
        let login: Url = "quilt://auth/callback?code=ABC123&host=quilt.test&state=xyz"
            .parse()
            .unwrap();
        assert_eq!(launch_page(LINK.into(), Some(&login)), LINK);
    }

    #[tokio::test]
    async fn a_navigation_goes_to_the_page_that_asked_or_else_the_window() {
        let data_dir = tempfile::tempdir().unwrap();
        let services = Services::init(data_dir.path().to_path_buf(), semver::Version::new(0, 0, 0))
            .await
            .unwrap();
        navigate(&services, Some("a-tab"), routes::Paths::Settings);
        navigate(&services, None, routes::Paths::Home);
        assert_eq!(
            services.navigations.pending("a-tab").unwrap().page,
            "/settings"
        );
        assert_eq!(
            services
                .navigations
                .pending(navigation::WINDOW)
                .unwrap()
                .page,
            "/"
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
            landing_after_login(Some("/installed-package?namespace=org%2Fpkg")),
            routes::Paths::InstalledPackage(("org", "pkg").into())
        );
        // A deep link's warning rides along in `back`; the page still parses.
        assert_eq!(
            landing_after_login(Some(
                "/installed-package?namespace=org%2Fpkg&mismatch=abc&mrbucket=b"
            )),
            routes::Paths::InstalledPackage(("org", "pkg").into())
        );
        assert_eq!(
            landing_after_login(Some("/commit?namespace=org%2Fpkg")),
            routes::Paths::Commit(("org", "pkg").into())
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
