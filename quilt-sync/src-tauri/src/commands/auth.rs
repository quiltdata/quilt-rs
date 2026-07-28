//! Login, login-error, OAuth, and auth-erase commands.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;
use tokio::sync;
use tokio::sync::OnceCell;

use quilt_rs::RoleInfo;
use quilt_uri::Host;

use crate::Error;
use crate::model;
use crate::model::QuiltModel;
use crate::notify::Notify;
use crate::oauth::OAuthState;
use crate::quilt;
use crate::routes;
use crate::telemetry::{MixpanelEvent, Telemetry, mixpanel::LoginFlow, prelude::*};

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

fn erase_auth_command(app_handle: &tauri::AppHandle, host: &str) -> Result<(), Error> {
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
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, Telemetry>,
    host: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::AuthErased).await;

    let app_handle = app_handle.lock().await;

    let msg_init = format!("Erasing auth for {host}");
    let msg_ok = format!("Successfully erased auth for {host}");
    let msg_err = |err: &Error| format!("Failed to erase auth: {err}");

    // Delete the on-disk token first, then invalidate the in-memory S3
    // client cache. A cached client holds STS credentials minted before
    // logout (valid ~1h), so without this the running app keeps serving
    // reads/writes until they expire.
    let result = erase_auth_command(&app_handle, &host);
    if result.is_ok() {
        // Global logout (empty host) clears every cached client; a per-host
        // logout clears only that host's. An unparseable non-empty host can
        // have no client keyed under it (cache keys are valid `Host`s), so
        // there is nothing to clear — and we must NOT fall back to clearing
        // everything, which would drop unrelated hosts' clients.
        if host.is_empty() {
            m.clear_remote_client_cache(None).await;
        } else if let Ok(host) = Host::from_str(&host) {
            m.clear_remote_client_cache(Some(host)).await;
        }
    }

    Notify::new(msg_init).map(result, msg_ok, msg_err)
}

// ── Role data for Leptos UI ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolesData {
    pub current: String,
    pub available: Vec<String>,
}

impl From<RoleInfo> for RolesData {
    fn from(info: RoleInfo) -> Self {
        Self {
            current: info.current,
            available: info.available,
        }
    }
}

/// Per-host [`RoleInfo`], resolved once and shared by everything that needs
/// to *name* the active role.
///
/// The roster is why this exists. Its two access-marking phases both want
/// the role, and the heavy phase runs as one Tauri command invocation per
/// row, concurrently — 50 denied rows on one host would otherwise queue 50
/// `/me` round trips behind `Auth::refresh_lock_for(host)`, the same mutex
/// credential vending takes, all re-answering a question the light phase
/// already answered.
///
/// Each key maps to an `Arc<OnceCell<..>>` rather than the value, so
/// concurrent misses on a host single-flight: the map lock is held only long
/// enough to get-or-insert the cell, and the fetch runs with it released.
/// A failed fetch leaves the cell uninitialised, so the next caller retries
/// rather than inheriting the failure. Same shape as
/// [`crate::commands::WorkflowRulesCache`].
#[derive(Default)]
pub struct RoleCache {
    roles: sync::Mutex<HashMap<Host, RoleCell>>,
}

type RoleCell = Arc<OnceCell<RoleInfo>>;

impl RoleCache {
    /// The active role on `host`, fetching it at most once per host.
    pub async fn get(&self, m: &impl QuiltModel, host: &Host) -> Result<RoleInfo, Error> {
        let cell = {
            let mut guard = self.roles.lock().await;
            Arc::clone(guard.entry(host.clone()).or_default())
        };
        let roles = cell.get_or_try_init(|| m.refresh_roles(host)).await?;
        Ok(roles.clone())
    }

    /// Forget `host`'s cached role, or every host's when `None`.
    ///
    /// A switch makes the cached name wrong, and a wrong role name in the
    /// roster's reason line is worse than no cache at all.
    pub async fn invalidate(&self, host: Option<&Host>) {
        let mut guard = self.roles.lock().await;
        match host {
            Some(host) => {
                guard.remove(host);
            }
            None => guard.clear(),
        }
    }
}

async fn get_roles_command(m: &impl QuiltModel, host: &str) -> Result<RolesData, Error> {
    let host = Host::from_str(host)?;
    Ok(RolesData::from(m.refresh_roles(&host).await?))
}

#[tauri::command]
pub async fn get_roles(
    m: tauri::State<'_, model::Model>,
    host: String,
) -> Result<RolesData, String> {
    get_roles_command(&*m, &host)
        .await
        .map_err(|e| e.to_string())
}

/// Switch the primary role, then make it take effect locally.
///
/// Several caches must go, not one. `switch_role` expires the stored STS
/// credentials, but an already-built S3 client holds its own copy in the
/// SDK's identity cache and would keep signing as the old role for up to
/// an hour. `erase_auth` has the same two-step for logout. The
/// [`RoleCache`] goes too, or the roster keeps quoting the previous role
/// back at the user.
///
/// Clear the cache only **after** the switch succeeds — clearing first
/// would let an in-flight vend repopulate it under the old role.
async fn switch_role_command(
    m: &impl QuiltModel,
    roles: &RoleCache,
    tracing: &Telemetry,
    host: &str,
    role: &str,
) -> Result<RolesData, Error> {
    let host = Host::from_str(host)?;
    let info = m.switch_role(&host, role).await?;
    let host_name = host.to_string();
    // Three caches, not two: the stored credentials (expired by the switch),
    // the S3 clients holding their own copy, and the role name the roster
    // quotes back at the user.
    roles.invalidate(Some(&host)).await;
    m.clear_remote_client_cache(Some(host)).await;

    tracing
        .track(MixpanelEvent::RoleSwitched { host: host_name })
        .await;

    Ok(RolesData::from(info))
}

#[tauri::command]
pub async fn switch_role(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, Telemetry>,
    host: String,
    role: String,
) -> Result<RolesData, String> {
    switch_role_command(&*m, &roles, &tracing, &host, &role)
        .await
        .map_err(|e| e.to_string())
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
    tracing: &Telemetry,
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
    tracing: tauri::State<'_, Telemetry>,
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
    tracing: tauri::State<'_, Telemetry>,
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
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::model::MockQuiltModel;

    /// A `MockQuiltModel` paired with the log of hosts whose cached S3
    /// clients were dropped, so a test can assert on the flush itself
    /// rather than on the switch's return value.
    struct RecordingModel {
        model: MockQuiltModel,
        cache_clears: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingModel {
        /// Hosts passed to `clear_remote_client_cache`, in call order. A
        /// global clear (`None`) is recorded as an empty string, mirroring
        /// `erase_auth`'s empty-host convention.
        fn cache_clears_for_host(&self) -> Vec<String> {
            self.cache_clears.lock().expect("cache clear log").clone()
        }
    }

    /// A model whose `switch_role` succeeds — confirming `confirmed_role`
    /// whatever was requested — and whose cache clears are recorded.
    fn mock_model_recording_cache_clears(confirmed_role: &str) -> RecordingModel {
        let cache_clears = Arc::new(Mutex::new(Vec::new()));
        let mut model = MockQuiltModel::new();

        let role = confirmed_role.to_string();
        model.expect_switch_role().returning(move |_, _| {
            Ok(RoleInfo {
                current: role.clone(),
                available: vec!["ReadWrite".to_string(), role.clone()],
            })
        });

        let log = Arc::clone(&cache_clears);
        model
            .expect_clear_remote_client_cache()
            .returning(move |host: Option<Host>| {
                log.lock()
                    .expect("cache clear log")
                    .push(host.map(|h| h.to_string()).unwrap_or_default());
            });

        RecordingModel {
            model,
            cache_clears,
        }
    }

    /// A switch must expire the stored credentials AND drop the cached S3
    /// clients. Dropping only the credentials leaves an already-built client
    /// signing with the old role until its own STS expiry (~1h), which is the
    /// feature silently doing nothing. Mirrors `erase_auth`'s contract.
    #[tokio::test]
    async fn switch_role_clears_the_remote_client_cache() {
        let recording = mock_model_recording_cache_clears("ReadOnly");
        let host = "test.quilt.dev";

        switch_role_command(
            &recording.model,
            &RoleCache::default(),
            &Telemetry::default(),
            host,
            "ReadOnly",
        )
        .await
        .expect("switch");

        assert_eq!(
            recording.cache_clears_for_host(),
            vec![host.to_string()],
            "a switch must clear the host's cached S3 clients, not just its credentials"
        );
    }

    /// The switch reports the role the server **confirmed**, never the one
    /// the UI asked for. A stack that normalises or falls back to a
    /// different role than requested would otherwise leave the UI naming
    /// one role while S3 signs as another. The requested spelling here
    /// (`readonly`) deliberately differs from the confirmed one
    /// (`ReadOnly`), so echoing the request back fails this test.
    #[tokio::test]
    async fn switch_role_returns_the_confirmed_role_not_the_requested_one() {
        let recording = mock_model_recording_cache_clears("ReadOnly");

        let data = switch_role_command(
            &recording.model,
            &RoleCache::default(),
            &Telemetry::default(),
            "test.quilt.dev",
            "readonly",
        )
        .await
        .expect("switch");

        assert_eq!(
            data.current, "ReadOnly",
            "the UI must show the role the stack confirmed, not the string it was sent"
        );
        assert_eq!(data.available, vec!["ReadWrite", "ReadOnly"]);
    }

    /// A rejected switch must leave the cached clients alone: the old role
    /// is still the active one, and dropping its clients would force a
    /// pointless re-vend on every in-flight request.
    #[tokio::test]
    async fn switch_role_leaves_the_cache_alone_when_the_switch_fails() {
        let cache_clears = Arc::new(Mutex::new(Vec::new()));
        let mut model = MockQuiltModel::new();
        model
            .expect_switch_role()
            .returning(|_, _| Err(Error::General("role rejected".to_string())));

        let log = Arc::clone(&cache_clears);
        model
            .expect_clear_remote_client_cache()
            .returning(move |host: Option<Host>| {
                log.lock()
                    .expect("cache clear log")
                    .push(host.map(|h| h.to_string()).unwrap_or_default());
            });

        let result = switch_role_command(
            &model,
            &RoleCache::default(),
            &Telemetry::default(),
            "test.quilt.dev",
            "ReadOnly",
        )
        .await;

        assert!(result.is_err(), "a rejected switch must surface the error");
        assert!(
            cache_clears.lock().expect("cache clear log").is_empty(),
            "a failed switch must not drop the still-valid clients of the current role"
        );
    }

    /// `get_roles` hands the stack's answer to the UI unchanged, so the
    /// selector opens on the role that is actually active.
    #[tokio::test]
    async fn get_roles_returns_the_active_role_and_the_choices() {
        let mut model = MockQuiltModel::new();
        model.expect_refresh_roles().returning(|_| {
            Ok(RoleInfo {
                current: "ReadWrite".to_string(),
                available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
            })
        });

        let data = get_roles_command(&model, "test.quilt.dev")
            .await
            .expect("roles");

        assert_eq!(data.current, "ReadWrite");
        assert_eq!(data.available, vec!["ReadWrite", "ReadOnly"]);
    }

    /// An unparseable host must fail before the remote call, so a typo
    /// surfaces as an error rather than as an empty role list.
    #[tokio::test]
    async fn get_roles_rejects_an_unparseable_host() {
        let mut model = MockQuiltModel::new();
        model.expect_refresh_roles().never();

        let result = get_roles_command(&model, "").await;

        assert!(result.is_err(), "an empty host is not a queryable host");
    }

    /// An unparseable host must fail before any remote call — `Host` is the
    /// cache key, so a switch we cannot key could never be flushed.
    #[tokio::test]
    async fn switch_role_rejects_an_unparseable_host() {
        let mut model = MockQuiltModel::new();
        model.expect_switch_role().never();
        model.expect_clear_remote_client_cache().never();

        let result = switch_role_command(
            &model,
            &RoleCache::default(),
            &Telemetry::default(),
            "",
            "ReadOnly",
        )
        .await;

        assert!(result.is_err(), "an empty host is not a switchable host");
    }

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
