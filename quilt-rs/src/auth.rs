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
use tracing::trace;
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
use retry::is_role_auth_error;
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

/// Valid STS credentials retained after the first disk read. The per-host
/// refresh lock owns the slow path; this cache only removes repeated parsing
/// on the normal S3 request path.
type CredentialCache = Arc<StdMutex<HashMap<Host, Credentials>>>;

/// Endpoint label for the role-surface retry logs. All three role calls
/// share one registry endpoint, so they share one name.
const ROLE_ENDPOINT: &str = "registry GraphQL endpoint";

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
    credential_cache: CredentialCache,
    session_roles: SessionRoles,
}

impl<S: Storage> Clone for Auth<S> {
    fn clone(&self) -> Self {
        Self {
            paths: self.paths.clone(),
            storage: Arc::clone(&self.storage),
            refresh_locks: Arc::clone(&self.refresh_locks),
            credential_cache: Arc::clone(&self.credential_cache),
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
            credential_cache: Arc::new(StdMutex::new(HashMap::new())),
            session_roles: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    fn cached_credentials(&self, host: &Host) -> Option<Credentials> {
        let mut cache = self
            .credential_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match cache.get(host) {
            Some(credentials) if credentials.expires_at > chrono::Utc::now() => {
                Some(credentials.clone())
            }
            Some(_) => {
                cache.remove(host);
                None
            }
            None => None,
        }
    }

    fn cache_credentials(&self, host: &Host, credentials: &Credentials) {
        self.credential_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(host.clone(), credentials.clone());
    }

    fn clear_cached_credentials(&self, host: &Host) {
        self.credential_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(host);
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

        let access_token = self
            .force_refresh_access_token(http_client, auth_io, host)
            .await?;

        classify_retry_outcome(
            self.refresh_credentials(http_client, host, &access_token)
                .await,
            is_credentials_auth_error,
            "credentials endpoint",
            host,
        )
    }

    /// Mint a new access token unconditionally, bypassing the 60s proactive
    /// window [`Auth::valid_access_token`] checks.
    ///
    /// Used on the retry leg of an auth failure: the server has already told
    /// us it does not accept the token we hold, so re-reading the same one
    /// would only reproduce the failure. Rotates and persists the refresh
    /// token, so callers must already hold this host's refresh lock.
    async fn force_refresh_access_token<T: HttpClient>(
        &self,
        http_client: &T,
        auth_io: &AuthIo<Arc<S>>,
        host: &Host,
    ) -> Res<String> {
        let tokens = auth_io
            .read_tokens()
            .await?
            .ok_or_else(|| LoginError::Required(Some(host.to_owned())))?;
        let new_tokens = self
            .refresh_tokens_with_retry(http_client, auth_io, host, &tokens)
            .await?;
        Ok(new_tokens.access_token)
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
        self.cache_credentials(host, &credentials);

        debug!(
            "✔️ Successfully refreshed credentials in {:?}",
            self.paths.auth_host(host)
        );
        Ok(credentials)
    }

    /// Expire this host's cached STS credentials once, keeping the login
    /// token. Narrower than logout: the session survives, and once the
    /// caller has done its half of the contract below, the next operation
    /// re-vends under whatever role the server now considers primary.
    ///
    /// # This is only half of a credential flush
    ///
    /// Deleting the on-disk credentials is **not sufficient on its own**.
    /// Any `aws_sdk_s3::Client` that was already built holds its own
    /// in-memory identity cache (`IdentityCache::lazy()`, installed by
    /// `aws_config::defaults`), which keeps the *previously vended* STS
    /// credentials until they approach their own expiry — up to about an
    /// hour. Such a client never re-reads this file, so it keeps signing
    /// under the old role no matter what happens on disk.
    ///
    /// Callers **must** therefore also call
    /// [`RemoteS3::clear_client_cache(Some(host))`] for the same host, or
    /// the old role stays in force until its natural expiry. `Auth` cannot
    /// do this itself: it does not own the S3 client cache, and the two are
    /// composed a layer up.
    ///
    /// [`RemoteS3::clear_client_cache(Some(host))`]: crate::io::remote::RemoteS3::clear_client_cache
    ///
    /// # Locking
    ///
    /// Takes this host's refresh lock, so a vend already in flight cannot
    /// write old-role credentials back over the gap. Internal callers that
    /// already hold the lock use the private `expire_credentials_locked`
    /// instead — the lock is not reentrant.
    pub async fn expire_credentials(&self, host: &Host) -> Res {
        let lock = self.refresh_lock_for(host);
        let _guard = lock.lock().await;
        self.expire_credentials_locked(host).await
    }

    /// [`Auth::expire_credentials`] without the lock, for callers already
    /// holding this host's refresh lock. Carries the same "clear the S3
    /// client cache too" obligation — see that method's docs.
    async fn expire_credentials_locked(&self, host: &Host) -> Res {
        info!("⏳ Expiring cached credentials for {}", host);
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.delete_credentials().await?;
        self.clear_cached_credentials(host);
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
    /// old-role credentials to disk after we removed them. That is also why
    /// this calls [`Auth::expire_credentials_locked`] and not the public,
    /// self-locking [`Auth::expire_credentials`] — the lock is not reentrant.
    async fn reconcile_role(&self, host: &Host, role: &str) -> Res {
        if !self.observe_role(host, role) {
            return Ok(());
        }

        info!(
            "⚠️ Active role for {} is {}, flushing credentials",
            host, role
        );
        self.expire_credentials_locked(host).await.inspect_err(|_| {
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

    /// Everything a role-surface call needs before it can be made: the
    /// registry host to talk to, and a locally-valid access token.
    ///
    /// Callers must already hold this host's refresh lock — resolving the
    /// token may rotate and persist the refresh token.
    async fn role_call_context<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<(url::Host, String)> {
        let access_token = self.valid_access_token(http_client, host).await?;
        let registry = get_registry_url(http_client, host).await?;
        Ok((registry, access_token))
    }

    /// Second half of the role-surface retry policy: given the error a role
    /// call just failed with, either mint a fresh access token to retry with
    /// or hand the error straight back.
    ///
    /// Locally-valid-but-revoked tokens are the case this exists for. The
    /// 60s proactive window in [`Auth::valid_access_token`] cannot see a
    /// session dropped server-side, so without a forced refresh the role
    /// surface would report a bare transport error and never self-heal —
    /// the same reasoning as [`Auth::refresh_credentials_with_retry`], whose
    /// machinery ([`classify_retry_outcome`]) the callers reuse for the
    /// second attempt.
    ///
    /// Callers must already hold this host's refresh lock; the lock is not
    /// reentrant.
    async fn role_retry_token<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        first_err: Error,
    ) -> Res<String> {
        if !is_role_auth_error(&first_err) {
            return Err(first_err);
        }

        info!(
            status = ?http_status(&first_err),
            "⚠️ Auth error on the role surface for {}, \
             force-refreshing token and retrying: {}",
            host, first_err
        );

        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        self.force_refresh_access_token(http_client, &auth_io, host)
            .await
    }

    /// Read the active role from the registry and reconcile the local
    /// credential cache with it. Any observed change expires this host's
    /// on-disk credentials.
    ///
    /// # The flush is only half done when this returns
    ///
    /// Expiring the on-disk credentials is **not sufficient on its own** to
    /// put the new role into force: an `aws_sdk_s3::Client` that already
    /// exists keeps its own in-memory identity cache and goes on signing
    /// with the old role's STS credentials for up to about an hour. The
    /// caller **must** also call [`RemoteS3::clear_client_cache(Some(host))`]
    /// whenever the returned role differs from the one it last saw — only
    /// then does the next operation actually re-vend under the current role.
    ///
    /// [`RemoteS3::clear_client_cache(Some(host))`]: crate::io::remote::RemoteS3::clear_client_cache
    ///
    /// # Locking
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

        let (registry, access_token) = self.role_call_context(http_client, host).await?;
        let me = match query_me(http_client, &registry, host, &access_token).await {
            Ok(me) => me,
            Err(first_err) => {
                let retry_token = self.role_retry_token(http_client, host, first_err).await?;
                classify_retry_outcome(
                    query_me(http_client, &registry, host, &retry_token).await,
                    is_role_auth_error,
                    ROLE_ENDPOINT,
                    host,
                )?
            }
        };

        // One line per *answered* `me` query, whether or not the role moved.
        // Without it the only role logging is `reconcile_role`'s flush, which
        // fires just on a *change* — so the ordinary case is silent and the
        // cadence (one query per host per roster load, never one per row)
        // cannot be observed at all. Keep it: this is the cheapest way to
        // tell "asked once" from "asked per package".
        //
        // A *failed* ask never reaches this line, and deliberately so. The
        // retry machinery reports those, and counting them here too would
        // mean restructuring this function so the outcome is a return value
        // rather than an early `?` — not worth it for an exact count of a
        // case the callers already report loudly.
        debug!("🔎 Asked {} for the active role: {}", host, me.role.name);

        self.reconcile_role(host, &me.role.name).await?;

        Ok(RoleInfo::from(me))
    }

    /// Make `role_name` the user's primary role. The change is server-side
    /// and global across all of that user's sessions; locally we expire the
    /// cached credentials so the next vend re-scopes.
    ///
    /// # The flush is only half done when this returns
    ///
    /// As with [`Auth::refresh_roles`], deleting the on-disk credentials is
    /// **not sufficient on its own**: an already-built `aws_sdk_s3::Client`
    /// holds its own in-memory identity cache and keeps signing under the
    /// role that was active when it last vended — for up to about an hour,
    /// no matter what this method removed from disk. Every caller of
    /// `switch_role` **must** also call
    /// [`RemoteS3::clear_client_cache(Some(host))`] for the same host;
    /// otherwise the switch returns `Ok` while every subsequent S3 call
    /// still runs as the old role. `Auth` cannot do it for you — it does
    /// not own the client cache.
    ///
    /// [`RemoteS3::clear_client_cache(Some(host))`]: crate::io::remote::RemoteS3::clear_client_cache
    ///
    /// # Locking
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

        let (registry, access_token) = self.role_call_context(http_client, host).await?;
        let me = match mutate_switch_role(http_client, &registry, role_name, &access_token).await {
            Ok(me) => me,
            Err(first_err) => {
                let retry_token = self.role_retry_token(http_client, host, first_err).await?;
                classify_retry_outcome(
                    mutate_switch_role(http_client, &registry, role_name, &retry_token).await,
                    is_role_auth_error,
                    ROLE_ENDPOINT,
                    host,
                )?
            }
        };

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
    /// Takes the refresh lock only because resolving the access token, and
    /// the retry leg behind it, may rotate the persisted refresh token; it
    /// flushes nothing itself.
    pub async fn readable_buckets<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<Vec<String>> {
        let lock = self.refresh_lock_for(host);
        let _guard = lock.lock().await;

        let (registry, access_token) = self.role_call_context(http_client, host).await?;
        match query_buckets(http_client, &registry, &access_token).await {
            Ok(buckets) => Ok(buckets),
            Err(first_err) => {
                let retry_token = self.role_retry_token(http_client, host, first_err).await?;
                classify_retry_outcome(
                    query_buckets(http_client, &registry, &retry_token).await,
                    is_role_auth_error,
                    ROLE_ENDPOINT,
                    host,
                )
            }
        }
    }

    pub async fn get_credentials_or_refresh<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<Credentials> {
        // `trace`: this is called on the path of every remote operation, so it
        // fires roughly once a second under autosync and the answer is almost
        // always "the cached ones are fine". The outcomes worth a line are the
        // *unusual* ones below — no credentials, or a refresh — which stay louder.
        trace!("⏳ Getting or refreshing credentials for {}", host);

        if let Some(creds) = self.cached_credentials(host) {
            trace!("✔️ Found valid in-memory credentials for {}", host);
            return Ok(creds);
        }

        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));

        match auth_io.read_credentials().await {
            Ok(Some(creds)) => {
                // A cache hit is the normal case and says nothing.
                self.cache_credentials(host, &creds);
                trace!("✔️ Found valid credentials for {}", host);
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

        if let Some(creds) = self.cached_credentials(host) {
            debug!("✔️ Another task refreshed credentials for {}", host);
            return Ok(creds);
        }

        match auth_io.read_credentials().await {
            Ok(Some(creds)) => {
                self.cache_credentials(host, &creds);
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
        self.cache_credentials(host, &creds);
        info!("✔️ Successfully refreshed credentials for {}", host);
        Ok(creds)
    }
}
