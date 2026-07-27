//! OAuth 2.1 Authorization Code flow with PKCE for Quilt catalog authentication.
//!
//! The [`Auth`] store orchestrates login, token refresh, and S3 credential
//! vending, persisting state per catalog host. The wire protocol lives in
//! the `oauth` submodule (RFC 6749/7591/7636 machinery against the connect
//! host, including the RFC terminology mapping) and the `registry` submodule
//! (registry API calls); `retry` classifies endpoint errors for the bounded
//! retry policy.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::Weak;

use tokio::sync::Mutex as AsyncMutex;

use crate::Error;
use crate::Res;
use crate::error::AuthError;
use crate::error::LoginError;
use crate::io::remote::client::HttpClient;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::io::storage::auth::AuthIo;
use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::OAuthClient;
use crate::io::storage::auth::Tokens;
use crate::paths::DomainPaths;
use quilt_uri::Host;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

mod graphql;
mod oauth;
mod registry;
mod retry;

pub use oauth::OAuthParams;
pub use oauth::PkceChallenge;
pub use oauth::catalog_authorize_url;
pub use oauth::connect_host;
pub use oauth::pkce_challenge;
pub use oauth::random_state;
pub use registry::RemoteTokens;

use graphql::mutate_switch_role;
use graphql::query_buckets;
use graphql::query_me;
use oauth::exchange_oauth_code;
use oauth::refresh_oauth_tokens;
use oauth::register_client;
use registry::get_auth_tokens;
use registry::get_registry_url;
use registry::refresh_credentials;
use retry::classify_retry_outcome;
use retry::http_status;
use retry::is_credentials_auth_error;
use retry::is_token_auth_error;

#[cfg(test)]
mod test_utils;
#[cfg(test)]
mod tests;

/// Map of per-host refresh locks used to single-flight concurrent
/// credential refreshes. The outer `StdMutex` is held only across the
/// brief map lookup and is never held across an `.await`. The inner
/// `AsyncMutex` is held across the HTTP refresh, serializing refreshes
/// for a single host.
///
/// Entries are `Weak`, so the map size tracks *in-flight* refreshes
/// rather than distinct hosts seen over the process lifetime. Racing
/// callers upgrade the same `Weak` and share the mutex; once everyone
/// drops their `Arc`, the entry becomes a dead `Weak` and is pruned
/// on the next lookup. This matters for long-running server contexts
/// that may authenticate against many distinct hosts.
type RefreshLocks = Arc<StdMutex<HashMap<Host, Weak<AsyncMutex<()>>>>>;

/// Per-host record of the active role observed *this session*. Purely
/// in-memory and never persisted: its absence for a host is meaningful —
/// it means any credentials on disk were vended by an unknown role, which
/// is why the first observation of a session always flushes.
type SessionRoles = Arc<StdMutex<HashMap<Host, String>>>;

/// The active role plus every role the user holds, as the switcher needs
/// them. `available` includes `current`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleInfo {
    pub current: String,
    pub available: Vec<String>,
}

impl From<graphql::Me> for RoleInfo {
    fn from(me: graphql::Me) -> Self {
        Self {
            current: me.role.name,
            available: me.roles.into_iter().map(|r| r.name).collect(),
        }
    }
}

#[derive(Debug)]
pub struct Auth<S: Storage = LocalStorage> {
    pub paths: DomainPaths,
    pub storage: Arc<S>,
    refresh_locks: RefreshLocks,
    session_roles: SessionRoles,
}

impl<S: Storage> Clone for Auth<S> {
    fn clone(&self) -> Self {
        Self {
            paths: self.paths.clone(),
            storage: Arc::clone(&self.storage),
            refresh_locks: Arc::clone(&self.refresh_locks),
            session_roles: Arc::clone(&self.session_roles),
        }
    }
}

impl<S: Storage + Send + Sync> Auth<S> {
    pub fn new(paths: DomainPaths, storage: Arc<S>) -> Self {
        Self {
            paths,
            storage,
            refresh_locks: Arc::new(StdMutex::new(HashMap::new())),
            session_roles: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Get the `Arc<Mutex>` for this host's refresh lock, creating it
    /// on first use. The outer lock is only held for the brief map
    /// lookup — never across `.await`. Dead `Weak` entries (mutex no
    /// longer referenced by any in-flight refresh) are swept before
    /// the lookup so the map stays bounded by active refreshes.
    fn refresh_lock_for(&self, host: &Host) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .refresh_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, weak| weak.strong_count() > 0);
        if let Some(arc) = locks.get(host).and_then(Weak::upgrade) {
            return arc;
        }
        let arc = Arc::new(AsyncMutex::new(()));
        locks.insert(host.clone(), Arc::downgrade(&arc));
        arc
    }

    pub async fn login<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        refresh_token: String,
    ) -> Res {
        info!("⏳ Logging in to host {} with refresh token", host);

        let tokens = match self
            .get_auth_tokens(http_client, host, &refresh_token)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                warn!("❌ Failed to get auth tokens for {}: {}", host, e);
                return Err(e);
            }
        };

        if let Err(e) = self.save_tokens(host, &tokens).await {
            warn!("❌ Failed to save tokens for {}: {}", host, e);
            return Err(e);
        }

        if let Err(e) = self
            .refresh_credentials(http_client, host, &tokens.access_token)
            .await
        {
            warn!("❌ Failed to refresh credentials for {}: {}", host, e);
            return Err(e);
        }

        info!("✔️ Successfully logged in and authenticated to {}", host);
        Ok(())
    }

    /// Get a stored OAuth `client_id` for the host, or register a new one via DCR.
    pub async fn get_or_register_client<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        redirect_uri: &str,
    ) -> Res<OAuthClient> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));

        if let Some(client) = auth_io.read_client().await? {
            if client.redirect_uri == redirect_uri {
                info!("✔️ Found existing OAuth client for {}", host);
                return Ok(client);
            }
            info!(
                "⚠️ Cached client has stale redirect_uri, re-registering for {}",
                host
            );
        }

        info!("⏳ Registering new OAuth client for {}", host);
        let client = register_client(http_client, host, redirect_uri).await?;
        auth_io.write_client(&client).await?;
        info!(
            "✔️ Registered OAuth client for {}: {}",
            host, client.client_id
        );

        Ok(client)
    }

    /// Login using OAuth 2.1 Authorization Code flow with PKCE.
    ///
    /// Exchanges the authorization code for tokens, then fetches S3 credentials.
    ///
    /// # State / CSRF verification
    ///
    /// This method does not verify the `state` parameter returned by the
    /// Authorization Endpoint. The caller is responsible for comparing the
    /// `state` value in the callback against the value generated by
    /// [`random_state`] before calling this method (RFC 6749 §10.12).
    pub async fn login_oauth<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        params: OAuthParams,
    ) -> Res {
        info!("⏳ OAuth login for host {}", host);

        let tokens = exchange_oauth_code(http_client, host, &params)
            .await
            .map_err(|e| {
                warn!("❌ Failed to exchange OAuth code for {}: {}", host, e);
                e
            })?;

        self.save_tokens(host, &tokens).await.map_err(|e| {
            warn!("❌ Failed to save tokens for {}: {}", host, e);
            e
        })?;

        self.refresh_credentials(http_client, host, &tokens.access_token)
            .await
            .map_err(|e| {
                warn!("❌ Failed to refresh credentials for {}: {}", host, e);
                e
            })?;

        info!("✔️ OAuth login successful for {}", host);
        Ok(())
    }

    async fn get_auth_tokens<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        refresh_token: &str,
    ) -> Res<Tokens> {
        debug!("⏳ Getting auth tokens for host {:?}", host);
        let tokens = get_auth_tokens(http_client, host, refresh_token).await?;
        debug!("✔️ Successfully retrieved auth tokens");
        Ok(tokens)
    }

    async fn save_tokens(&self, host: &Host, tokens: &Tokens) -> Res<()> {
        debug!("⏳ Saving tokens for host {:?}", host);
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_tokens(tokens).await?;
        debug!(
            "✔️ Successfully saved tokens to the {:?}",
            self.paths.auth_host(host)
        );
        Ok(())
    }

    /// Use the refresh token to obtain new access + refresh tokens from the
    /// Connect token endpoint (RFC 6749 §6), then persist them.
    async fn refresh_tokens<T: HttpClient>(
        &self,
        http_client: &T,
        auth_io: &AuthIo<Arc<S>>,
        host: &Host,
        tokens: &Tokens,
    ) -> Res<Tokens> {
        let client = auth_io
            .read_client()
            .await?
            .ok_or(LoginError::Required(Some(host.to_owned())))?;

        let new_tokens =
            refresh_oauth_tokens(http_client, host, &tokens.refresh_token, &client.client_id)
                .await?;

        auth_io.write_tokens(&new_tokens).await?;
        info!("✔️ Successfully refreshed tokens for {}", host);

        Ok(new_tokens)
    }

    /// `refresh_tokens` with a single transparent retry on auth-classified
    /// errors (HTTP 400/401/403 from the token endpoint).
    ///
    /// A single 4xx is not necessarily a revoked refresh token — it can also
    /// be a brief server-side token-validation hiccup (deploy, replica with
    /// stale state, JWKS rotation). Only when two consecutive attempts return
    /// a 4xx do we conclude the refresh token is actually bad and map to
    /// `LoginError::Required`.
    async fn refresh_tokens_with_retry<T: HttpClient>(
        &self,
        http_client: &T,
        auth_io: &AuthIo<Arc<S>>,
        host: &Host,
        tokens: &Tokens,
    ) -> Res<Tokens> {
        let first_err = match self
            .refresh_tokens(http_client, auth_io, host, tokens)
            .await
        {
            Ok(t) => return Ok(t),
            Err(e) => e,
        };

        if matches!(first_err, Error::Login(LoginError::Required(_))) {
            warn!("❌ No OAuth client registered for {}, login required", host);
            return Err(first_err);
        }
        if !is_token_auth_error(&first_err) {
            warn!(
                status = ?http_status(&first_err),
                "❌ Failed to refresh tokens for {}: {}", host, first_err
            );
            return Err(first_err);
        }

        info!(
            status = ?http_status(&first_err),
            "⚠️ Auth error refreshing tokens for {}, retrying once: {}", host, first_err
        );
        classify_retry_outcome(
            self.refresh_tokens(http_client, auth_io, host, tokens)
                .await,
            is_token_auth_error,
            "token endpoint",
            host,
        )
    }

    /// `refresh_credentials` with a single transparent retry on auth-classified
    /// errors (HTTP 401/403 from the credentials endpoint).
    ///
    /// A 4xx here usually means the server's view of the access token's
    /// validity has shifted (clock skew, session-store replication lag, etc.).
    /// Unlike the token-endpoint retry, this path **forces** a fresh access
    /// token between the two attempts — retrying with the same stale token
    /// would just reproduce the failure.
    async fn refresh_credentials_with_retry<T: HttpClient>(
        &self,
        http_client: &T,
        auth_io: &AuthIo<Arc<S>>,
        host: &Host,
        access_token: &str,
    ) -> Res<Credentials> {
        let first_err = match self
            .refresh_credentials(http_client, host, access_token)
            .await
        {
            Ok(c) => return Ok(c),
            Err(e) => e,
        };

        if !is_credentials_auth_error(&first_err) {
            warn!(
                status = ?http_status(&first_err),
                "❌ Failed to refresh credentials for {}: {}", host, first_err
            );
            return Err(first_err);
        }

        info!(
            status = ?http_status(&first_err),
            "⚠️ Auth error refreshing credentials for {}, \
             force-refreshing token and retrying: {}",
            host, first_err
        );

        // Force-refresh the access token, bypassing the 60s proactive check.
        let tokens = auth_io
            .read_tokens()
            .await?
            .ok_or_else(|| LoginError::Required(Some(host.to_owned())))?;
        let new_tokens = self
            .refresh_tokens_with_retry(http_client, auth_io, host, &tokens)
            .await?;

        classify_retry_outcome(
            self.refresh_credentials(http_client, host, &new_tokens.access_token)
                .await,
            is_credentials_auth_error,
            "credentials endpoint",
            host,
        )
    }

    async fn refresh_credentials<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        access_token: &str,
    ) -> Res<Credentials> {
        debug!("⏳ Refreshing credentials for host {:?}", host);
        let credentials = refresh_credentials(http_client, host, access_token).await?;

        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_credentials(&credentials).await?;

        debug!(
            "✔️ Successfully refreshed credentials in {:?}",
            self.paths.auth_host(host)
        );
        Ok(credentials)
    }

    /// Expire this host's cached STS credentials once, keeping the login
    /// token. Narrower than logout: the session survives and the next
    /// operation re-vends, picking up whatever role the server now
    /// considers primary.
    pub async fn expire_credentials(&self, host: &Host) -> Res {
        info!("⏳ Expiring cached credentials for {}", host);
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.delete_credentials().await?;
        info!("✔️ Cached credentials expired for {}", host);
        Ok(())
    }

    /// Record the role just observed for this host and report whether the
    /// credential cache is now stale. `insert` returns the previous value:
    /// `None` means this is the session's first observation (disk creds are
    /// role-unknown), `Some(prev)` differing means the role changed under us.
    fn observe_role(&self, host: &Host, role: &str) -> bool {
        let mut roles = self
            .session_roles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !matches!(roles.insert(host.clone(), role.to_owned()), Some(prev) if prev == role)
    }

    /// Return this host to the role-*unknown* state, so the next observation
    /// flushes no matter what role it sees. Used both to roll back a failed
    /// flush and to force one on an explicit switch.
    fn forget_role(&self, host: &Host) {
        self.session_roles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(host);
    }

    /// Reconcile the session's role baseline with `role`, flushing the
    /// credential cache if the role changed (or was never observed).
    ///
    /// The baseline is only allowed to stand once the flush has actually
    /// succeeded. Recording it first would let a failed flush convince every
    /// later observation that nothing had changed, stranding old-role
    /// credentials for the rest of the session with no way to recover; so on
    /// failure we drop back to role-unknown and the next call retries.
    ///
    /// Callers must hold this host's refresh lock: the delete has to be
    /// serialized against an in-flight vend, which would otherwise write
    /// old-role credentials to disk after we removed them.
    async fn reconcile_role(&self, host: &Host, role: &str) -> Res {
        if !self.observe_role(host, role) {
            return Ok(());
        }

        info!(
            "⚠️ Active role for {} is {}, flushing credentials",
            host, role
        );
        self.expire_credentials(host).await.inspect_err(|_| {
            warn!(
                "❌ Flush for {} failed; keeping the role unknown so the next \
                 observation retries",
                host
            );
            self.forget_role(host);
        })
    }

    /// A currently-valid access token for `host`, refreshing it first if it
    /// is inside the 60s expiry window.
    ///
    /// Rotates and persists the refresh token when it does refresh, so every
    /// caller must already hold this host's refresh lock. Deliberately does
    /// not take the lock itself — `get_credentials_or_refresh` calls this
    /// while holding the guard, and the lock is not reentrant.
    async fn valid_access_token<T: HttpClient>(&self, http_client: &T, host: &Host) -> Res<String> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        let tokens = auth_io
            .read_tokens()
            .await?
            .ok_or_else(|| LoginError::Required(Some(host.to_owned())))?;

        if tokens.expires_at <= chrono::Utc::now() + chrono::Duration::seconds(60) {
            info!("⏳ Access token expired for {}, refreshing", host);
            let refreshed = self
                .refresh_tokens_with_retry(http_client, &auth_io, host, &tokens)
                .await?;
            return Ok(refreshed.access_token);
        }
        Ok(tokens.access_token)
    }

    /// Read the active role from the registry and reconcile the local
    /// credential cache with it. Any observed change expires this host's
    /// credentials so the next operation re-vends under the current role.
    ///
    /// Runs under this host's refresh lock, which buys two things: the token
    /// refresh cannot race a concurrent vend into rotating the stored refresh
    /// token twice, and the flush cannot land in the middle of a vend that
    /// would then write old-role credentials over the gap.
    pub async fn refresh_roles<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<RoleInfo> {
        let lock = self.refresh_lock_for(host);
        let _guard = lock.lock().await;

        let access_token = self.valid_access_token(http_client, host).await?;
        let registry = get_registry_url(http_client, host).await?;
        let me = query_me(http_client, &registry, host, &access_token).await?;

        self.reconcile_role(host, &me.role.name).await?;

        Ok(RoleInfo::from(me))
    }

    /// Make `role_name` the user's primary role. The change is server-side
    /// and global across all of that user's sessions; locally we only have
    /// to expire the cached credentials so the next vend re-scopes.
    ///
    /// Holds this host's refresh lock for the same reasons as
    /// [`Auth::refresh_roles`] — most of all so a sync already vending
    /// credentials cannot write the old role's credentials back after the
    /// switch has flushed them.
    pub async fn switch_role<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        role_name: &str,
    ) -> Res<RoleInfo> {
        info!("⏳ Switching role on {} to {}", host, role_name);
        let lock = self.refresh_lock_for(host);
        let _guard = lock.lock().await;

        let access_token = self.valid_access_token(http_client, host).await?;
        let registry = get_registry_url(http_client, host).await?;
        let me = mutate_switch_role(http_client, &registry, role_name, &access_token).await?;

        // A switch always invalidates, even when the server reports the role
        // we were already on: dropping the baseline first makes the
        // reconcile below see an unknown role and flush unconditionally.
        self.forget_role(host);
        self.reconcile_role(host, &me.role.name).await?;

        info!("✔️ Switched role on {} to {}", host, me.role.name);
        Ok(RoleInfo::from(me))
    }

    /// The buckets the active role can read. An optimistic hint, not an
    /// authoritative answer — see the doc comment on the `buckets` query.
    ///
    /// Takes the refresh lock only because [`Auth::valid_access_token`] may
    /// rotate the persisted refresh token; it flushes nothing itself.
    pub async fn readable_buckets<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<Vec<String>> {
        let lock = self.refresh_lock_for(host);
        let _guard = lock.lock().await;

        let access_token = self.valid_access_token(http_client, host).await?;
        let registry = get_registry_url(http_client, host).await?;
        query_buckets(http_client, &registry, &access_token).await
    }

    pub async fn get_credentials_or_refresh<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<Credentials> {
        info!("⏳ Getting or refreshing credentials for {}", host);
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));

        match auth_io.read_credentials().await {
            Ok(Some(creds)) => {
                debug!("✔️ Found valid credentials for {}", host);
                return Ok(creds);
            }
            Ok(None) => {
                info!("❌ No existing credentials found for {}", host);
            }
            Err(e) => {
                error!("❌ Failed to read credentials for {}: {}", host, e);
                return Err(Error::Auth(
                    host.to_owned(),
                    AuthError::CredentialsRead(e.to_string()),
                ));
            }
        }

        // Serialize refreshes for this host so N concurrent callers
        // fire one HTTP `/get_credentials` call instead of N. The
        // loser of the race re-reads the credentials the winner
        // wrote to disk and returns them without hitting the network.
        let lock = self.refresh_lock_for(host);
        let _guard = lock.lock().await;

        match auth_io.read_credentials().await {
            Ok(Some(creds)) => {
                debug!("✔️ Another task refreshed credentials for {}", host);
                return Ok(creds);
            }
            Ok(None) => {}
            Err(e) => {
                error!("❌ Failed to re-read credentials for {}: {}", host, e);
                return Err(Error::Auth(
                    host.to_owned(),
                    AuthError::CredentialsRead(e.to_string()),
                ));
            }
        }

        // Read the tokens up front purely to classify the two failure modes
        // this path promises: a missing token file means login is required,
        // an unreadable one is a storage fault. `valid_access_token` re-reads
        // them for the value itself.
        match auth_io.read_tokens().await {
            Ok(Some(_)) => {}
            Ok(None) => {
                warn!("❌ No tokens found for {}, login required", host);
                return Err(LoginError::Required(Some(host.to_owned())).into());
            }
            Err(e) => {
                error!("❌ Failed to read tokens for {}: {}", host, e);
                return Err(Error::Auth(
                    host.to_owned(),
                    AuthError::TokensRead(e.to_string()),
                ));
            }
        }

        let access_token = self.valid_access_token(http_client, host).await?;

        info!("⏳ Refreshing credentials using access token for {}", host);
        let creds = self
            .refresh_credentials_with_retry(http_client, &auth_io, host, &access_token)
            .await?;
        info!("✔️ Successfully refreshed credentials for {}", host);
        Ok(creds)
    }
}
