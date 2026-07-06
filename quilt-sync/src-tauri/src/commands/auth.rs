//! Login, login-error, OAuth, and auth-erase commands.

use std::str::FromStr;

use serde::Serialize;
use tauri::Manager;
use tokio::sync;

use crate::Error;
use crate::model;
use crate::notify::Notify;
use crate::oauth::OAuthState;
use crate::quilt;
use crate::routes;
use crate::telemetry::{MixpanelEvent, mixpanel::LoginFlow, prelude::*};

// ── Login data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub host: String,
    pub back: String,
    pub catalog_url: String,
}

#[tauri::command]
pub async fn get_login_data(host: String, back: String) -> Result<LoginData, String> {
    let catalog_url = format!("https://{host}/code");
    Ok(LoginData {
        host,
        back,
        catalog_url,
    })
}

// ── Login error data for Leptos UI ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginErrorData {
    pub title: String,
    pub message: String,
    pub login_host: String,
}

#[tauri::command]
pub async fn get_login_error_data(
    host: String,
    title: Option<String>,
    error: String,
) -> Result<LoginErrorData, String> {
    Ok(LoginErrorData {
        title: title.unwrap_or_else(|| "Login failed".into()),
        message: error,
        login_host: host,
    })
}

async fn erase_auth_command(app_handle: &tauri::AppHandle, host: &str) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let auth_dir = local_data_dir.join(quilt::paths::AUTH_DIR);

    if host.is_empty() {
        // Global erase (backward compat)
        if auth_dir.exists() {
            std::fs::remove_dir_all(&auth_dir)?;
        }
    } else {
        // Per-host erase — canonicalize and verify containment
        let host_dir = auth_dir.join(host);
        if host_dir.exists() {
            let canonical = host_dir.canonicalize()?;
            let canonical_auth = auth_dir.canonicalize()?;
            if !canonical.starts_with(&canonical_auth) {
                return Err(Error::General(format!("Invalid host: {host}")));
            }
            std::fs::remove_dir_all(&canonical)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn erase_auth(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::AuthErased).await;

    let app_handle = app_handle.lock().await;

    let msg_init = format!("Erasing auth for {host}");
    let msg_ok = format!("Successfully erased auth for {host}");
    let msg_err = |err: &Error| format!("Failed to erase auth: {err}");

    Notify::new(msg_init).map(
        erase_auth_command(&app_handle, &host).await,
        msg_ok,
        msg_err,
    )
}

/// Navigate to a page after successful login.
pub(crate) fn navigate_after_login(
    app_handle: &tauri::AppHandle,
    path: routes::Paths,
) -> Result<(), Error> {
    debug!("Attempting to redirect after login to: {:?}", path);
    let win = app_handle
        .get_webview_window("main")
        .ok_or(crate::error::TauriUiError::Window)?;
    let win_url = win.url()?;
    let redirect_url = routes::from_url(path, win_url);
    debug!("Redirecting to: {}", redirect_url);
    win.navigate(redirect_url)?;
    Ok(())
}

/// Code-based login for legacy stacks that don't support Connect/OAuth.
async fn login_command(
    m: &model::Model,
    tracing: &crate::telemetry::Telemetry,
    host: &str,
    code: String,
) -> Result<(), Error> {
    let host = quilt_uri::Host::from_str(host)?;
    model::login(m, &host, code).await?;

    tracing
        .track(MixpanelEvent::UserLoggedIn {
            host: host.to_string(),
            flow: LoginFlow::Legacy,
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn login(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
    code: String,
) -> Result<String, String> {
    let msg_init = format!("Login with code for host {host}");
    let msg_ok = format!("Successfully logged in to {host}");
    let msg_err = |err: &Error| format!("Failed to login: {err}");

    Notify::new(msg_init).map(
        login_command(&m, &tracing, &host, code).await,
        msg_ok,
        msg_err,
    )
}

/// Initiate OAuth 2.1 login: register client via DCR if needed,
/// generate PKCE, store verifier, open browser.
#[tauri::command]
pub async fn login_oauth(
    m: tauri::State<'_, model::Model>,
    oauth_state: tauri::State<'_, OAuthState>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
    back: Option<String>,
) -> Result<String, String> {
    let host_parsed = quilt_uri::Host::from_str(&host).map_err(|e| e.to_string())?;

    let redirect_uri = crate::oauth::redirect_uri(&host_parsed);
    let client_id = model::get_or_register_client(&*m, &host_parsed, &redirect_uri)
        .await
        .map_err(|e| e.to_string())?;

    let request = oauth_state
        .start_login(&host_parsed, &client_id, back)
        .await;

    model::open_in_web_browser(&request.authorize_url).map_err(|e| e.to_string())?;

    tracing
        .track(MixpanelEvent::OAuthLoginInitiated { host: host.clone() })
        .await;

    Ok(format!("Opening browser for OAuth login to {host}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_login_error_data() -> Result<(), String> {
        let data = get_login_error_data(
            "test.quilt.dev".to_string(),
            Some("Login failed".to_string()),
            "Auth failed".to_string(),
        )
        .await?;
        assert_eq!(data.title, "Login failed");
        assert_eq!(data.message, "Auth failed");
        assert_eq!(data.login_host, "test.quilt.dev");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_login_error_data_default_title() -> Result<(), String> {
        let data = get_login_error_data(
            "test.quilt.dev".to_string(),
            None,
            "Auth failed".to_string(),
        )
        .await?;
        assert_eq!(data.title, "Login failed");
        assert_eq!(data.message, "Auth failed");
        Ok(())
    }

    // ── Login data tests ──
    // (Adapted from pages/login.rs: test_login_page_rendering)

    #[tokio::test]
    async fn test_get_login_data() -> Result<(), String> {
        let data = get_login_data(
            "test.quilt.dev".to_string(),
            "/installed-packages-list".to_string(),
        )
        .await?;

        assert_eq!(data.host, "test.quilt.dev");
        assert_eq!(data.back, "/installed-packages-list");
        assert_eq!(data.catalog_url, "https://test.quilt.dev/code");
        Ok(())
    }

    // (Adapted from pages/login.rs: test_login_oauth_button_without_back)

    #[tokio::test]
    async fn test_get_login_data_empty_back() -> Result<(), String> {
        let data = get_login_data("test.quilt.dev".to_string(), String::new()).await?;

        assert_eq!(data.host, "test.quilt.dev");
        assert_eq!(data.back, "");
        assert_eq!(data.catalog_url, "https://test.quilt.dev/code");
        Ok(())
    }
}
