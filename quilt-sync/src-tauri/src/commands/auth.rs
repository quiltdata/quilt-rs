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
use crate::autopull::Watcher;
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
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, Telemetry>,
    host: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::AuthErased).await;

    let app_handle = app_handle.lock().await;

    let msg_init = format!("Erasing auth for {host}");
    let msg_ok = format!("Successfully erased auth for {host}");
    let msg_err = |err: &Error| format!("Failed to erase auth: {err}");

    let result = erase_auth_command(&app_handle, &host);
    if result.is_ok() {
        erase_role_state(&*m, &roles, &host).await;
    }

    Notify::new(msg_init).map(result, msg_ok, msg_err)
}

/// Drop everything a logged-out session left behind that names or empowers a
/// role, once the tokens are already off disk.
///
/// Two caches, for two different failure modes. The S3 clients hold STS
/// credentials minted *before* the logout and valid for about an hour, so
/// without dropping them the app keeps serving reads and writes to someone
/// who has signed out. The [`RoleCache`] holds the role's *name* and the list
/// of roles that user held; it outlives the process's login, so logging back
/// in as a different person leaves the roster quoting a stranger's role and
/// offering the switch affordance on their entitlements.
///
/// Global logout (empty host) clears every host; a per-host logout clears
/// only that host's. An unparseable non-empty host can have nothing keyed
/// under it — both caches key on a valid [`Host`] — so there is nothing to
/// clear, and we must NOT fall back to clearing everything, which would drop
/// unrelated hosts.
async fn erase_role_state(m: &impl QuiltModel, roles: &RoleCache, host: &str) {
    if host.is_empty() {
        roles.invalidate(None).await;
        m.clear_remote_client_cache(None).await;
    } else if let Ok(host) = Host::from_str(host) {
        roles.invalidate(Some(&host)).await;
        m.clear_remote_client_cache(Some(host)).await;
    }
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

/// Per-host [`RoleInfo`], resolved once per roster load and shared by
/// everything that needs to *name* the active role.
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
    /// The active role on `host`, fetching it at most once per host between
    /// invalidations.
    ///
    /// Not once per session: the roster load invalidates every host before it
    /// paints, so the entry lives for one list load. That is the cadence the
    /// refresh is pinned to — long enough that fifty denied rows on one host
    /// share a single answer, short enough that a switch made in the web
    /// catalog surfaces on the next load rather than at the ~1h credential
    /// expiry.
    ///
    /// The fetch goes through [`observe_role`], because reading the role is
    /// not a read: whenever the registry names a role this session has not
    /// seen, the engine expires the host's stored credentials, and finishing
    /// that flush is the caller's half of the contract. This is a real
    /// observation path — a role switched in the web catalog first shows up
    /// here, on the roster's mark for a bucket that just started denying.
    pub async fn get(&self, m: &impl QuiltModel, host: &Host) -> Result<RoleInfo, Error> {
        let cell = {
            let mut guard = self.roles.lock().await;
            Arc::clone(guard.entry(host.clone()).or_default())
        };
        // Not `adopt_role`: the cell being initialised *is* this host's cache
        // entry, so publishing the value is `get_or_try_init`'s own job.
        let roles = cell.get_or_try_init(|| observe_role(m, host)).await?;
        Ok(roles.clone())
    }

    /// Publish `info` as `host`'s role, replacing whatever was cached.
    ///
    /// Used when a caller has just been told the role by the registry, so the
    /// next reader gets the new name without another round trip.
    async fn set(&self, host: &Host, info: RoleInfo) {
        self.roles
            .lock()
            .await
            .insert(host.clone(), Arc::new(OnceCell::new_with(Some(info))));
    }

    /// Forget `host`'s cached role, or every host's when `None`.
    ///
    /// A switch makes the cached name wrong, and a wrong role name in the
    /// roster's reason line is worse than no cache at all — which is why the
    /// roster load clears every host before it paints, since a switch made in
    /// the web catalog leaves us nothing to notice. Logout goes
    /// further: the next user is a different person, and quoting the previous
    /// one's role — or gating the switch affordance on their `available` list
    /// — would outlive the session that earned it.
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

/// Read the active role for `host`, completing the flush the read starts.
///
/// `refresh_roles` is **not a pure read**. The engine keeps a per-session
/// baseline of the role it last saw on each host, and whenever the registry
/// answers with a different one — the user switched in the web catalog, say,
/// which is legitimate because a switch is server-side and global — it
/// deletes that host's stored STS credentials. That is only half a flush:
/// an `aws_sdk_s3::Client` that already exists holds its own copy of the old
/// role's credentials in the SDK's identity cache, never re-reads the file,
/// and goes on signing as the old role for up to an hour. Dropping those
/// clients is the other half, and only this layer owns them.
///
/// Skipping it produces the worst possible state: the selector shows the new
/// role, the registry-backed surfaces agree, and every real S3 call is still
/// the old one — with no way back, because re-picking the role the UI already
/// displays is a no-op.
///
/// The drop is unconditional. Only the engine knows whether the role moved,
/// and guessing from up here is exactly the mistake this exists to prevent;
/// the cost of being wrong is one client rebuild and a re-read of the
/// credentials still on disk.
async fn observe_role(m: &impl QuiltModel, host: &Host) -> Result<RoleInfo, Error> {
    let info = m.refresh_roles(host).await?;
    m.clear_remote_client_cache(Some(host.clone())).await;
    Ok(info)
}

/// Adopt `info` as the role now in force on `host`.
///
/// Two caches, and both have to move together: the [`RoleCache`] the roster
/// quotes back at the user, and the S3 clients still signing as the role it
/// replaced. Every caller that learns a role from the registry — the explicit
/// switch and the read paths that can observe an out-of-band one — goes
/// through here, so a new caller cannot land with only one of the two.
async fn adopt_role(m: &impl QuiltModel, roles: &RoleCache, host: &Host, info: RoleInfo) {
    roles.set(host, info).await;
    m.clear_remote_client_cache(Some(host.clone())).await;
}

pub(super) async fn get_roles_command(
    m: &impl QuiltModel,
    roles: &RoleCache,
    host: &str,
) -> Result<RolesData, Error> {
    let host = Host::from_str(host)?;
    // Settings is where an out-of-band switch usually surfaces, so this is a
    // deliberately uncached read — and therefore an observation that has to
    // be followed through. See [`observe_role`].
    let info = m.refresh_roles(&host).await?;
    adopt_role(m, roles, &host, info.clone()).await;
    Ok(RolesData::from(info))
}

#[tauri::command]
pub async fn get_roles(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    host: String,
) -> Result<RolesData, String> {
    get_roles_command(&*m, &roles, &host)
        .await
        .map_err(|e| e.to_string())
}

/// Switch the primary role, then make it take effect locally.
///
/// Several caches must go, not one. `switch_role` expires the stored STS
/// credentials, but an already-built S3 client holds its own copy in the
/// SDK's identity cache and would keep signing as the old role for up to
/// an hour. `erase_auth` has the same two-step for logout. The
/// [`RoleCache`] moves too, or the roster keeps quoting the previous role
/// back at the user. [`adopt_role`] is the one place that knows all of them.
///
/// Autosync's role-denied pauses are released here as well: they are the one
/// pause the user cannot clear by hand, because every manual action that
/// clears a pause is signed by the role that was refused. A switch is the
/// remedy the banner names, so it has to be the remedy that works.
///
/// Move the caches only **after** the switch succeeds — moving them first
/// would let an in-flight vend repopulate them under the old role.
pub(super) async fn switch_role_command(
    m: &impl QuiltModel,
    roles: &RoleCache,
    watcher: &Watcher,
    tracing: &Telemetry,
    host: &str,
    role: &str,
) -> Result<RolesData, Error> {
    let host = Host::from_str(host)?;
    let info = m.switch_role(&host, role).await?;
    let host_name = host.to_string();
    // Three caches, not two: the stored credentials (expired by the switch),
    // the S3 clients holding their own copy, and the role name the roster
    // quotes back at the user. The last two are `adopt_role`'s job.
    adopt_role(m, roles, &host, info.clone()).await;
    watcher.clear_role_denied_pauses().await;

    tracing
        .track(MixpanelEvent::RoleSwitched { host: host_name })
        .await;

    Ok(RolesData::from(info))
}

#[tauri::command]
pub async fn switch_role(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    watcher: tauri::State<'_, Watcher>,
    tracing: tauri::State<'_, Telemetry>,
    host: String,
    role: String,
) -> Result<RolesData, String> {
    switch_role_command(&*m, &roles, &watcher, &tracing, &host, &role)
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
    use crate::autopull::PausedReason;
    use crate::autopull::reporter::LogReporter;
    use crate::model::MockQuiltModel;
    use quilt_uri::Namespace;

    /// A watcher with no background task, for the switch's pause release.
    fn test_watcher() -> Watcher {
        Watcher::new_for_test(Arc::new(LogReporter))
    }

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
            &test_watcher(),
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

    /// Autosync parks a package whose bucket the role cannot reach, and
    /// tells the user to switch role to resume. Nothing else releases that
    /// pause — a manual push or pull is signed by the same refused role — so
    /// if the switch does not release it, the package stays parked until the
    /// app restarts and the advice on the banner is false.
    #[tokio::test]
    async fn switch_role_resumes_autosync_for_role_denied_packages() {
        let recording = mock_model_recording_cache_clears("ReadWrite");
        let watcher = test_watcher();
        let namespace: Namespace = ("acme", "demo").into();
        watcher
            .pause_for_test(
                namespace.clone(),
                PausedReason::RoleDenied {
                    role: "ReadOnly".to_string(),
                },
            )
            .await;

        switch_role_command(
            &recording.model,
            &RoleCache::default(),
            &watcher,
            &Telemetry::default(),
            "test.quilt.dev",
            "ReadWrite",
        )
        .await
        .expect("switch");

        assert!(
            watcher.snapshot().await.paused.is_empty(),
            "the switch the banner asks for must let autosync try again"
        );
    }

    /// A failed switch changes nothing, so the pause must survive it: the
    /// old role is still active and still denied.
    #[tokio::test]
    async fn a_rejected_switch_leaves_the_denial_pause_in_place() {
        let mut model = MockQuiltModel::new();
        model
            .expect_switch_role()
            .returning(|_, _| Err(Error::General("role not held".to_string())));
        let watcher = test_watcher();
        let namespace: Namespace = ("acme", "demo").into();
        watcher
            .pause_for_test(
                namespace.clone(),
                PausedReason::RoleDenied {
                    role: "ReadOnly".to_string(),
                },
            )
            .await;

        let result = switch_role_command(
            &model,
            &RoleCache::default(),
            &watcher,
            &Telemetry::default(),
            "test.quilt.dev",
            "ReadWrite",
        )
        .await;

        assert!(result.is_err(), "a rejected switch must surface the error");
        assert_eq!(
            watcher.snapshot().await.paused.len(),
            1,
            "nothing changed, so the denial still stands"
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
            &test_watcher(),
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
            &test_watcher(),
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

    /// A user holding two roles, the second of which is active.
    fn two_roles() -> RoleInfo {
        RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
        }
    }

    /// A `RoleCache` already holding `host`'s role, as it would be after any
    /// roster paint that hit a denial.
    async fn warmed_role_cache(host: &Host) -> RoleCache {
        let mut model = MockQuiltModel::new();
        model
            .expect_refresh_roles()
            .times(1)
            .returning(|_| Ok(two_roles()));
        model.expect_clear_remote_client_cache().returning(|_| ());

        let roles = RoleCache::default();
        assert_eq!(
            roles.get(&model, host).await.expect("roles").current,
            "ReadOnly"
        );
        roles
    }

    /// Logging out has to drop the role too, not just the credentials.
    ///
    /// The `RoleCache` is app-lifetime and holds the previous user's role
    /// *name* and the list of roles they held. Log out, log in as someone
    /// else, and the roster quotes a stranger's role at the new user and
    /// gates the switch affordance on entitlements they do not have — for as
    /// long as the process lives.
    #[tokio::test]
    async fn logging_out_drops_the_cached_role() {
        let host: Host = "test.quilt.dev".parse().expect("host");
        let roles = warmed_role_cache(&host).await;

        let mut erasing = MockQuiltModel::new();
        erasing.expect_clear_remote_client_cache().returning(|_| ());
        erase_role_state(&erasing, &roles, "test.quilt.dev").await;

        let mut next_user = MockQuiltModel::new();
        next_user.expect_refresh_roles().times(1).returning(|_| {
            Ok(RoleInfo {
                current: "Curator".to_string(),
                available: vec!["Curator".to_string()],
            })
        });
        next_user
            .expect_clear_remote_client_cache()
            .returning(|_| ());

        assert_eq!(
            roles.get(&next_user, &host).await.expect("roles").current,
            "Curator",
            "the next login must be asked afresh, not served the last user's role"
        );
    }

    /// A global logout (empty host) is not scoped to one catalog, so nothing
    /// may survive it.
    #[tokio::test]
    async fn a_global_logout_drops_every_cached_role() {
        let host: Host = "test.quilt.dev".parse().expect("host");
        let roles = warmed_role_cache(&host).await;

        let mut erasing = MockQuiltModel::new();
        erasing.expect_clear_remote_client_cache().returning(|_| ());
        erase_role_state(&erasing, &roles, "").await;

        let mut next_user = MockQuiltModel::new();
        next_user
            .expect_refresh_roles()
            .times(1)
            .returning(|_| Ok(two_roles()));
        next_user
            .expect_clear_remote_client_cache()
            .returning(|_| ());

        roles.get(&next_user, &host).await.expect("roles");
    }

    /// An unparseable, non-empty host keys nothing in either cache, so it
    /// must clear nothing — falling back to a global clear would log every
    /// other catalog out of its role.
    #[tokio::test]
    async fn an_unparseable_logout_host_clears_nothing() {
        let host: Host = "test.quilt.dev".parse().expect("host");
        let roles = warmed_role_cache(&host).await;

        let mut erasing = MockQuiltModel::new();
        erasing.expect_clear_remote_client_cache().never();
        erase_role_state(&erasing, &roles, "not a host").await;

        let mut untouched = MockQuiltModel::new();
        untouched.expect_refresh_roles().never();

        assert_eq!(
            roles.get(&untouched, &host).await.expect("roles").current,
            "ReadOnly",
            "an unkeyable host must leave the other hosts' entries alone"
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
        model.expect_clear_remote_client_cache().returning(|_| ());

        let data = get_roles_command(&model, &RoleCache::default(), "test.quilt.dev")
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
        model.expect_clear_remote_client_cache().never();

        let result = get_roles_command(&model, &RoleCache::default(), "").await;

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
            &test_watcher(),
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
