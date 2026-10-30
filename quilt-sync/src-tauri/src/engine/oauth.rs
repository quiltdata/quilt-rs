use std::collections::HashMap;
use std::time::Duration;

use url::Url;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::engine::Error;
use crate::engine::quilt;
use crate::engine::telemetry::prelude::*;

// `Duration::from_mins(10)` would read better, but is gated on Rust 1.91;
// workspace MSRV is 1.85.
#[allow(clippy::duration_suboptimal_units)]
const TTL: Duration = Duration::from_secs(10 * 60);

/// How long an expired login is kept, so its callback can still tell the page
/// that asked that it expired.
#[allow(clippy::duration_suboptimal_units)]
const FORGET: Duration = Duration::from_secs(24 * 60 * 60);

/// Pending OAuth authorization state, keyed by its `state` token, so two
/// logins to one host each land on their own page.
///
/// When the user initiates OAuth login, we generate a PKCE challenge
/// and store the verifier here. When the deep link callback arrives,
/// we look up the verifier by its state and exchange the code.
#[derive(Default)]
pub struct OAuthState {
    pending: Mutex<HashMap<String, PendingAuth>>,
}

struct PendingAuth {
    host: String,
    code_verifier: String,
    redirect_uri: String,
    client_id: String,
    landing: Landing,
    created_at: Instant,
}

/// Where a login lands once it succeeds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Landing {
    /// The page to show, as the login page was asked to return to.
    pub back: Option<String>,
    /// The id of the page that asked; `None` sends the window.
    pub page: Option<String>,
}

/// The URL and related data needed to open the browser for OAuth login.
pub struct AuthorizeRequest {
    pub authorize_url: String,
}

/// The redirect URI for `QuiltSync` OAuth callbacks.
///
/// The Connect server redirects here after the user authorizes.
/// The `quilt://` scheme is registered as a deep link, so the OS
/// routes the callback to `QuiltSync` where `uri::login_with_code` handles it.
pub fn redirect_uri(host: &quilt_uri::Host) -> String {
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
        host: &quilt_uri::Host,
        client_id: &str,
        landing: Landing,
    ) -> AuthorizeRequest {
        let pkce = quilt::auth::pkce_challenge();
        let redirect_uri = redirect_uri(host);
        let state = quilt::auth::random_state();

        let base = quilt::auth::catalog_authorize_url(host);
        let mut url = Url::parse(&base).unwrap_or_else(|_| {
            // catalog_authorize_url always returns a valid URL; this is a safeguard.
            Url::parse("https://invalid").expect("fallback URL is valid")
        });
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &pkce.code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("response_type", "code")
            .append_pair("scope", "platform")
            .append_pair("state", &state);
        let authorize_url = url.into();

        let host_key = host.to_string();
        let pending = PendingAuth {
            host: host_key.clone(),
            code_verifier: pkce.code_verifier,
            redirect_uri,
            client_id: client_id.to_string(),
            landing,
            created_at: Instant::now(),
        };

        let mut guard = self.pending.lock().await;
        guard.retain(|_, v| v.created_at.elapsed() < FORGET);
        guard.insert(state, pending);

        info!("Stored pending OAuth state for {host_key}");

        AuthorizeRequest { authorize_url }
    }

    /// Handle the Authorization Response (RFC 6749 §4.1.2) and verify
    /// the `state` parameter for CSRF protection (RFC 6749 §10.12).
    ///
    /// Returns:
    /// - `Ok(params)` — state matched, proceed with Token Request
    /// - `Err(_)` — no pending state for this host, state expired, or state
    ///   mismatched; abort, do not fall back
    pub async fn take_params(
        &self,
        host: &quilt_uri::Host,
        code: String,
        state: &str,
    ) -> (Landing, Result<quilt::auth::OAuthParams, Error>) {
        let host_key = host.to_string();
        let mut guard = self.pending.lock().await;
        let pending = match guard.remove(state) {
            Some(p) if p.host != host_key => {
                warn!("OAuth state for {} came back for {host_key}", p.host);
                return (
                    Landing::default(),
                    Err(Error::OAuthUi(crate::engine::error::OAuthUiError::OAuth(
                        format!("State mismatch for {host_key}: possible CSRF attack"),
                    ))),
                );
            }
            Some(p) if p.created_at.elapsed() < TTL => {
                info!("Found pending OAuth state for {host_key}");
                p
            }
            Some(expired) => {
                warn!("Pending OAuth state for {host_key} has expired");
                return (
                    expired.landing,
                    Err(Error::OAuthUi(crate::engine::error::OAuthUiError::OAuth(
                        format!(
                            "OAuth state for {host_key} has expired; login again to restart the flow"
                        ),
                    ))),
                );
            }
            None if guard.values().any(|p| p.host == host_key) => {
                warn!("OAuth state mismatch for {host_key}: possible CSRF attack");
                return (
                    Landing::default(),
                    Err(Error::OAuthUi(crate::engine::error::OAuthUiError::OAuth(
                        format!("State mismatch for {host_key}: possible CSRF attack"),
                    ))),
                );
            }
            None => {
                warn!("No pending OAuth state for {host_key}");
                return (
                    Landing::default(),
                    Err(Error::OAuthUi(crate::engine::error::OAuthUiError::OAuth(
                        format!(
                            "No active OAuth flow for {host_key}; \
                         please start the login flow from within the app"
                        ),
                    ))),
                );
            }
        };
        drop(guard);

        (
            pending.landing,
            Ok(quilt::auth::OAuthParams {
                code,
                code_verifier: pending.code_verifier,
                redirect_uri: pending.redirect_uri,
                client_id: pending.client_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quilt_uri::fixtures;

    #[test]
    fn redirect_uri_encodes_host() {
        let host = fixtures::host();
        assert_eq!(redirect_uri(&host), "quilt://auth/callback?host=quilt.test");
    }

    #[test]
    fn redirect_uri_encodes_ipv6_host() {
        let host: quilt_uri::Host = "[::1]".parse().unwrap();
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
        let host = fixtures::host();
        let landing = Landing {
            back: Some("/settings".to_string()),
            page: Some("a-tab".to_string()),
        };
        let req = oauth.start_login(&host, "client-id", landing).await;
        let state = extract_state(&req.authorize_url);
        let (landing, params) = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert!(params.is_ok());
        assert_eq!(
            landing,
            Landing {
                back: Some("/settings".to_string()),
                page: Some("a-tab".to_string()),
            },
            "the login lands where, and for whom, it was asked"
        );
    }

    #[tokio::test]
    async fn take_params_errors_for_unknown_host() {
        let oauth = OAuthState::default();
        let host = fixtures::host();
        // No start_login call — simulates an unsolicited deep link.
        let (landing, result) = oauth
            .take_params(&host, "auth-code".to_string(), "state")
            .await;
        assert_eq!(landing, Landing::default(), "no page asked for it");
        assert!(
            matches!(
                result,
                Err(Error::OAuthUi(crate::engine::error::OAuthUiError::OAuth(_)))
            ),
            "expected Err(OAuth) for unsolicited callback"
        );
    }

    #[tokio::test]
    async fn take_params_errors_after_ttl() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = fixtures::host();
        let asked = Landing {
            back: None,
            page: Some("a-tab".to_string()),
        };
        let req = oauth.start_login(&host, "client-id", asked).await;
        let state = extract_state(&req.authorize_url);
        tokio::time::advance(TTL + Duration::from_secs(1)).await;
        let (landing, result) = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert_eq!(
            landing.page.as_deref(),
            Some("a-tab"),
            "the page that asked hears that it expired"
        );
        assert!(
            matches!(
                result,
                Err(Error::OAuthUi(crate::engine::error::OAuthUiError::OAuth(_)))
            ),
            "expected Err(OAuth) for expired state"
        );
    }

    #[tokio::test]
    async fn an_expired_login_still_reaches_its_page_after_another_starts() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = fixtures::host();
        let asked = Landing {
            back: None,
            page: Some("a-tab".to_string()),
        };
        let req = oauth.start_login(&host, "client-id", asked).await;
        let state = extract_state(&req.authorize_url);
        tokio::time::advance(TTL + Duration::from_secs(1)).await;
        oauth
            .start_login(&fixtures::another_host(), "client-id", Landing::default())
            .await;

        let (landing, result) = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert_eq!(landing.page.as_deref(), Some("a-tab"));
        assert!(result.is_err(), "it expired");
    }

    #[tokio::test]
    async fn start_login_forgets_logins_after_a_day() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        oauth
            .start_login(&fixtures::one_host(), "client-id", Landing::default())
            .await;
        tokio::time::advance(FORGET + Duration::from_secs(1)).await;
        oauth
            .start_login(&fixtures::another_host(), "client-id", Landing::default())
            .await;

        let guard = oauth.pending.lock().await;
        assert_eq!(guard.len(), 1);
        assert!(guard.values().all(|p| p.host == "another.quilt.test"));
    }

    #[tokio::test]
    async fn two_logins_to_one_host_each_land_on_their_page() {
        tokio::time::pause();
        let oauth = OAuthState::default();
        let host = fixtures::host();
        let first = oauth
            .start_login(
                &host,
                "client-id",
                Landing {
                    back: None,
                    page: Some("first".to_string()),
                },
            )
            .await;
        let second = oauth
            .start_login(
                &host,
                "client-id",
                Landing {
                    back: None,
                    page: Some("second".to_string()),
                },
            )
            .await;

        for (req, page) in [(first, "first"), (second, "second")] {
            let state = extract_state(&req.authorize_url);
            let (landing, result) = oauth
                .take_params(&host, "auth-code".to_string(), &state)
                .await;
            assert!(result.is_ok());
            assert_eq!(landing.page.as_deref(), Some(page));
        }
    }

    #[tokio::test]
    async fn a_wrong_state_is_refused_and_lands_nowhere_in_particular() {
        let oauth = OAuthState::default();
        let host = fixtures::host();
        let asked = Landing {
            back: None,
            page: Some("a-tab".to_string()),
        };
        let req = oauth.start_login(&host, "client-id", asked).await;
        let (landing, result) = oauth
            .take_params(&host, "auth-code".to_string(), "forged")
            .await;
        assert_eq!(landing, Landing::default());
        assert!(result.is_err());

        let state = extract_state(&req.authorize_url);
        let (landing, result) = oauth
            .take_params(&host, "auth-code".to_string(), &state)
            .await;
        assert!(result.is_ok(), "the real callback still works");
        assert_eq!(landing.page.as_deref(), Some("a-tab"));
    }
}
