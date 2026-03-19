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
///
/// Also tracks hosts for which a login page was recently shown, so
/// legacy device-flow callbacks can be validated against it.
#[derive(Default)]
pub struct OAuthState {
    pending: Mutex<HashMap<String, PendingAuth>>,
    active_login_hosts: Mutex<HashMap<String, Instant>>,
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
    format!(
        "quilt://auth/callback?host={}",
        urlencoding::encode(&host.to_string())
    )
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
    /// - `Ok(None)` — no pending state for this host (legacy device flow callback)
    /// - `Err(_)` — state expired or mismatched; abort, do not fall back
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
                return Err(Error::OAuth(format!(
                    "OAuth state for {host_key} has expired; login again to restart the flow"
                )));
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

    /// Record that the login page was shown for `host`.
    ///
    /// Call this whenever the login page is rendered so that
    /// [`Self::is_login_host_active`] can validate legacy device-flow
    /// callbacks and reject unsolicited deep links.
    pub async fn record_login_host(&self, host: &quilt::uri::Host) {
        let host_key = host.to_string();
        let mut guard = self.active_login_hosts.lock().await;
        guard.retain(|_, v| v.elapsed() < TTL);
        guard.insert(host_key.clone(), Instant::now());
        debug!("Recorded active login host: {host_key}");
    }

    /// Returns `true` if the login page was shown for `host` within the TTL.
    pub async fn is_login_host_active(&self, host: &quilt::uri::Host) -> bool {
        let host_key = host.to_string();
        let guard = self.active_login_hosts.lock().await;
        guard.get(&host_key).map_or(false, |t| t.elapsed() < TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> quilt::uri::Host {
        "test.quilt.dev".parse().unwrap()
    }

    #[test]
    fn redirect_uri_encodes_host() {
        let host = test_host();
        assert_eq!(
            redirect_uri(&host),
            "quilt://auth/callback?host=test.quilt.dev"
        );
    }

    #[test]
    fn redirect_uri_encodes_ipv6_host() {
        let host: quilt::uri::Host = "[::1]".parse().unwrap();
        assert_eq!(
            redirect_uri(&host),
            "quilt://auth/callback?host=%5B%3A%3A1%5D"
        );
    }

    /// Extract the `state` parameter from the authorization URL returned by
    /// `start_login`, so tests can pass it back to `take_params`.
    fn extract_state(authorize_url: &str) -> String {
        let encoded = authorize_url
            .split("&state=")
            .nth(1)
            .expect("state param missing");
        urlencoding::decode(encoded)
            .expect("state is not valid percent-encoding")
            .into_owned()
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
    async fn take_params_errors_after_ttl() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = test_host();
        let req = oauth.start_login(&host, "client-id", None).await;
        let state = extract_state(&req.authorize_url);
        tokio::time::advance(TTL + Duration::from_secs(1)).await;
        let result = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert!(
            matches!(result, Err(Error::OAuth(_))),
            "expected Err(OAuth) for expired state"
        );
    }

    #[tokio::test]
    async fn start_login_evicts_expired_entries() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host_a: quilt::uri::Host = "host-a.quilt.dev".parse().unwrap();
        let host_b: quilt::uri::Host = "host-b.quilt.dev".parse().unwrap();

        // Login for host A — will expire.
        oauth.start_login(&host_a, "client-id", None).await;

        // Advance past TTL, then start a login for host B.
        // The retain in start_login should evict the stale host A entry.
        tokio::time::advance(TTL + Duration::from_secs(1)).await;
        oauth.start_login(&host_b, "client-id", None).await;

        // Only host B should remain; the expired host A entry must be gone.
        let guard = oauth.pending.lock().await;
        assert_eq!(guard.len(), 1);
        assert!(guard.contains_key("host-b.quilt.dev"));
    }
}
