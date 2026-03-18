use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::quilt;
use crate::telemetry::prelude::*;
use crate::Error;

const TTL: Duration = Duration::from_secs(10 * 60);

/// Pending OAuth authorization state, keyed by host.
///
/// When the user initiates OAuth login, we generate a PKCE challenge
/// and store the verifier here. When the deep link callback arrives,
/// we look up the verifier for the host and exchange the code.
#[derive(Default)]
pub struct OAuthState {
    pending: Mutex<HashMap<String, PendingAuth>>,
}

struct PendingAuth {
    code_verifier: String,
    redirect_uri: String,
    client_id: String,
    state: String,
    location: Option<String>,
    created_at: Instant,
}

/// The URL and related data needed to open the browser for OAuth login.
pub struct AuthorizeRequest {
    pub authorize_url: String,
}

/// The redirect URI for QuiltSync OAuth callbacks.
///
/// The Connect server redirects here after the user authorizes.
/// The `quilt://` scheme is registered as a deep link, so the OS
/// routes the callback to QuiltSync where `uri::login_with_code` handles it.
pub fn redirect_uri(host: &quilt::uri::Host) -> String {
    format!("quilt://auth/callback?host={host}")
}

impl OAuthState {
    /// Build an Authorization Request (RFC 6749 §4.1.1) with PKCE (RFC 7636 §4.3).
    ///
    /// Generates a PKCE challenge and a `state` token, stores them as
    /// pending auth, and returns the authorization URL to open in the browser.
    pub async fn start_login(
        &self,
        host: &quilt::uri::Host,
        client_id: &str,
        location: Option<String>,
    ) -> AuthorizeRequest {
        let pkce = quilt::auth::pkce_challenge();
        let redirect_uri = redirect_uri(host);
        let state = quilt::auth::random_state();

        let base = quilt::auth::catalog_authorize_url(host);
        let authorize_url = format!(
            "{base}?\
             client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &code_challenge={challenge}\
             &code_challenge_method=S256\
             &response_type=code\
             &scope=platform\
             &state={state}",
            client_id = urlencoding::encode(client_id),
            redirect_uri = urlencoding::encode(&redirect_uri),
            challenge = urlencoding::encode(&pkce.code_challenge),
            state = urlencoding::encode(&state),
        );

        let pending = PendingAuth {
            code_verifier: pkce.code_verifier,
            redirect_uri,
            client_id: client_id.to_string(),
            state: state.clone(),
            location,
            created_at: Instant::now(),
        };

        let host_key = host.to_string();
        let mut guard = self.pending.lock().await;
        guard.retain(|_, v| v.created_at.elapsed() < TTL);
        guard.insert(host_key.clone(), pending);

        info!("Stored pending OAuth state for {host_key}");

        AuthorizeRequest { authorize_url }
    }

    /// Handle the Authorization Response (RFC 6749 §4.1.2) and verify
    /// the `state` parameter for CSRF protection (RFC 6749 §10.12).
    ///
    /// Returns:
    /// - `Ok(Some(params))` — state matched, proceed with Token Request
    /// - `Ok(None)` — no pending state for this host (device flow callback)
    /// - `Err(_)` — state mismatch, possible CSRF attack; abort
    pub async fn take_params(
        &self,
        host: &quilt::uri::Host,
        code: String,
        state: &str,
    ) -> Result<Option<(quilt::auth::OAuthParams, Option<String>)>, Error> {
        let host_key = host.to_string();
        let mut guard = self.pending.lock().await;
        let pending = match guard.remove(&host_key) {
            Some(p) if p.created_at.elapsed() < TTL => {
                info!("Found pending OAuth state for {host_key}");
                p
            }
            Some(_) => {
                warn!("Pending OAuth state for {host_key} has expired");
                return Ok(None);
            }
            None => {
                let keys: Vec<String> = guard.keys().cloned().collect();
                warn!("No pending OAuth state for {host_key}. Pending hosts: {keys:?}");
                return Ok(None);
            }
        };
        drop(guard);

        if pending.state != state {
            warn!("OAuth state mismatch for {host_key}: possible CSRF attack");
            return Err(Error::OAuth(format!(
                "State mismatch for {host_key}: possible CSRF attack"
            )));
        }

        Ok(Some((
            quilt::auth::OAuthParams {
                code,
                code_verifier: pending.code_verifier,
                redirect_uri: pending.redirect_uri,
                client_id: pending.client_id,
            },
            pending.location,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> quilt::uri::Host {
        "test.quilt.dev".parse().unwrap()
    }

    /// Extract the `state` parameter from the authorization URL returned by
    /// `start_login`, so tests can pass it back to `take_params`.
    fn extract_state(authorize_url: &str) -> String {
        authorize_url
            .split("&state=")
            .nth(1)
            .expect("state param missing")
            .to_string()
    }

    #[tokio::test]
    async fn take_params_succeeds_immediately() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = test_host();
        let req = oauth.start_login(&host, "client-id", None).await;
        let state = extract_state(&req.authorize_url);
        let result = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert!(result.unwrap().is_some());
    }

    #[tokio::test]
    async fn take_params_returns_none_after_ttl() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = test_host();
        let req = oauth.start_login(&host, "client-id", None).await;
        let state = extract_state(&req.authorize_url);
        tokio::time::advance(TTL + Duration::from_secs(1)).await;
        let result = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn start_login_evicts_expired_entries() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = test_host();

        // First login — will expire.
        oauth.start_login(&host, "client-id", None).await;

        // Advance past TTL, then start a second login which should evict the first.
        tokio::time::advance(TTL + Duration::from_secs(1)).await;
        let req2 = oauth.start_login(&host, "client-id", None).await;
        let state2 = extract_state(&req2.authorize_url);

        // The map should now have exactly one entry (the fresh one).
        let guard = oauth.pending.lock().await;
        assert_eq!(guard.len(), 1);
        assert!(guard.values().next().unwrap().state == urlencoding::decode(&state2).unwrap());
    }
}
