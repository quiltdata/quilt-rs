use std::collections::HashMap;

use tokio::sync::Mutex;

use crate::quilt;
use crate::telemetry::prelude::*;

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
    /// Start an OAuth login flow for the given host.
    ///
    /// Generates a PKCE challenge, stores the verifier, and returns
    /// the authorization URL to open in the browser.
    pub async fn start_login(&self, host: &quilt::uri::Host, client_id: &str) -> AuthorizeRequest {
        let pkce = quilt::auth::pkce_challenge();
        let redirect_uri = redirect_uri(host);
        let state = quilt::auth::random_state();

        let authorize_url = format!(
            "https://{host}/connect/authorize?\
             client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &code_challenge={challenge}\
             &code_challenge_method=S256\
             &response_type=code\
             &scope=platform\
             &state={state}",
            challenge = pkce.code_challenge,
        );

        let pending = PendingAuth {
            code_verifier: pkce.code_verifier,
            redirect_uri,
            client_id: client_id.to_string(),
        };

        let host_key = host.to_string();
        self.pending.lock().await.insert(host_key.clone(), pending);

        info!("Stored pending OAuth state for {host_key}");

        AuthorizeRequest { authorize_url }
    }

    /// Complete an OAuth login by exchanging the authorization code.
    ///
    /// Returns `None` if there is no pending auth for the host
    /// (e.g. the user never initiated login or it was already consumed).
    pub async fn take_params(
        &self,
        host: &quilt::uri::Host,
        code: String,
    ) -> Option<quilt::auth::OAuthParams> {
        let host_key = host.to_string();
        let mut guard = self.pending.lock().await;
        let pending = match guard.remove(&host_key) {
            Some(p) => {
                info!("Found pending OAuth state for {host_key}");
                p
            }
            None => {
                let keys: Vec<String> = guard.keys().cloned().collect();
                warn!("No pending OAuth state for {host_key}. Pending hosts: {keys:?}");
                return None;
            }
        };
        drop(guard);

        Some(quilt::auth::OAuthParams {
            code,
            code_verifier: pending.code_verifier,
            redirect_uri: pending.redirect_uri,
            client_id: pending.client_id,
        })
    }
}
