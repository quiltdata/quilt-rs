use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::Digest;
use sha2::Sha256;

use crate::error::AuthError;
use crate::io::remote::client::HttpClient;
use crate::io::storage::auth::AuthIo;
use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::OAuthClient;
use crate::io::storage::auth::Tokens;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::paths::DomainPaths;
use crate::uri::Host;
use crate::Error;
use crate::Res;
use chrono::serde::ts_seconds;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

/// Parameters for the OAuth 2.1 Authorization Code flow with PKCE.
pub struct OAuthParams {
    /// The authorization code from the callback
    pub code: String,
    /// The PKCE code verifier generated before the auth request
    pub code_verifier: String,
    /// The redirect URI used in the authorization request
    pub redirect_uri: String,
    /// The OAuth client ID
    pub client_id: String,
}

/// PKCE code verifier and challenge pair (RFC 7636).
pub struct PkceChallenge {
    /// Random verifier string — send to token endpoint
    pub code_verifier: String,
    /// S256 hash of verifier — send in the authorize URL
    pub code_challenge: String,
}

/// Generate a PKCE code verifier and its S256 challenge.
///
/// The verifier is 64 random bytes, base64url-encoded (86 characters),
/// well within RFC 7636 §4.1's 43–128 character range.
pub fn pkce_challenge() -> PkceChallenge {
    let mut random_bytes = [0u8; 64];
    getrandom::fill(&mut random_bytes).expect("failed to generate random bytes");

    let code_verifier = URL_SAFE_NO_PAD.encode(random_bytes);
    let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));

    PkceChallenge {
        code_verifier,
        code_challenge,
    }
}

/// Generate a random state string for OAuth CSRF protection.
pub fn random_state() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("failed to generate random bytes");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Derive the connect server hostname from the catalog host.
///
/// The connect hostname is `<stack>-connect.<domain>`, where `<stack>` is the
/// first label of the catalog hostname.
///
/// E.g., `test.quilt.dev` → `test-connect.quilt.dev`
pub fn connect_host(host: &Host) -> String {
    let s = host.to_string();
    match s.split_once('.') {
        Some((stack, domain)) => format!("{stack}-connect.{domain}"),
        None => format!("{s}-connect"),
    }
}

/// Derive the connect server token endpoint from the catalog host.
///
/// E.g., `test.quilt.dev` → `https://test-connect.quilt.dev/auth/token`
fn connect_token_url(host: &Host) -> String {
    format!("https://{}/auth/token", connect_host(host))
}

/// Derive the connect server registration endpoint from the catalog host.
fn connect_register_url(host: &Host) -> String {
    format!("https://{}/auth/register", connect_host(host))
}

/// DCR request body (RFC 7591).
#[derive(Serialize)]
struct DcrRequest {
    client_name: String,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: String,
}

/// DCR response body (subset of fields we need).
#[derive(Deserialize, Serialize)]
struct DcrResponse {
    client_id: String,
}

/// Register a public OAuth client via Dynamic Client Registration.
async fn register_client(
    http_client: &impl HttpClient,
    host: &Host,
    redirect_uri: &str,
) -> Res<OAuthClient> {
    let register_url = connect_register_url(host);

    let request = DcrRequest {
        client_name: "QuiltSync".to_string(),
        redirect_uris: vec![redirect_uri.to_string()],
        token_endpoint_auth_method: "none".to_string(),
    };

    let response: DcrResponse = http_client.post_json(&register_url, &request).await?;

    Ok(OAuthClient {
        client_id: response.client_id,
        redirect_uri: redirect_uri.to_string(),
    })
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RemoteTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(with = "ts_seconds")]
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl From<RemoteTokens> for Tokens {
    fn from(raw: RemoteTokens) -> Self {
        Tokens {
            access_token: raw.access_token,
            refresh_token: raw.refresh_token,
            expires_at: raw.expires_at,
        }
    }
}

/// Token response from the Connect OAuth token endpoint.
///
/// Uses `expires_in` (seconds until expiry) per RFC 6749,
/// unlike `RemoteTokens` which uses `expires_at` (Unix timestamp).
#[derive(Deserialize, Serialize, Debug)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

impl From<OAuthTokenResponse> for Tokens {
    fn from(raw: OAuthTokenResponse) -> Self {
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(raw.expires_in);
        Tokens {
            access_token: raw.access_token,
            refresh_token: raw.refresh_token,
            expires_at,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct RemoteCredentials {
    access_key_id: String,
    #[serde(deserialize_with = "date_from_rfc3339")]
    expiration: chrono::DateTime<chrono::Utc>,
    secret_access_key: String,
    session_token: String,
}

impl From<RemoteCredentials> for Credentials {
    fn from(raw: RemoteCredentials) -> Self {
        Credentials {
            access_key: raw.access_key_id,
            secret_key: raw.secret_access_key,
            token: raw.session_token,
            expires_at: raw.expiration,
        }
    }
}

fn date_from_rfc3339<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<chrono::DateTime<chrono::Utc>, D::Error> {
    use serde::de::Error;
    String::deserialize(deserializer).and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .map_err(|e| Error::custom(format!("Invalid RFC3339 date: {e}")))
            .map(|dt| dt.with_timezone(&chrono::Utc))
    })
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct QuiltStackConfig {
    registry_url: url::Url,
}

async fn get_registry_url(http_client: &impl HttpClient, host: &Host) -> Res<url::Host> {
    let QuiltStackConfig { registry_url } = http_client
        .get(&format!("https://{host}/config.json"), None)
        .await?;
    Ok(url::Host::Domain(
        registry_url
            .domain()
            .ok_or(crate::Error::LoginRequiredRegistryUrl(host.to_owned()))?
            .to_string(),
    ))
}

async fn get_auth_tokens(
    http_client: &impl HttpClient,
    host: &Host,
    refresh_token: &str,
) -> Res<Tokens> {
    let registry = get_registry_url(http_client, host).await?;

    let mut form_data: HashMap<String, String> = HashMap::new();
    form_data.insert("refresh_token".to_string(), refresh_token.to_string());
    let tokens_json: RemoteTokens = http_client
        .post(&format!("https://{registry}/api/token"), &form_data)
        .await?;
    let tokens = Tokens::from(tokens_json);

    Ok(tokens)
}

async fn exchange_oauth_code(
    http_client: &impl HttpClient,
    host: &Host,
    params: &OAuthParams,
) -> Res<Tokens> {
    let token_url = connect_token_url(host);

    let mut form_data: HashMap<String, String> = HashMap::new();
    form_data.insert("grant_type".to_string(), "authorization_code".to_string());
    form_data.insert("code".to_string(), params.code.clone());
    form_data.insert("code_verifier".to_string(), params.code_verifier.clone());
    form_data.insert("redirect_uri".to_string(), params.redirect_uri.clone());
    form_data.insert("client_id".to_string(), params.client_id.clone());

    let tokens_json: OAuthTokenResponse = http_client.post(&token_url, &form_data).await?;
    Ok(Tokens::from(tokens_json))
}

async fn refresh_credentials(
    http_client: &impl HttpClient,
    host: &Host,
    access_token: &str,
) -> Res<Credentials> {
    let registry = get_registry_url(http_client, host).await?;

    let creds_json: RemoteCredentials = http_client
        .get(
            &format!("https://{registry}/api/auth/get_credentials"),
            Some(access_token),
        )
        .await?;

    let credentials = Credentials::from(creds_json);

    Ok(credentials)
}

#[derive(Debug, Clone)]
pub struct Auth<S: Storage = LocalStorage> {
    pub paths: DomainPaths,
    pub storage: S,
}

impl<S: Storage + Sync + Clone> Auth<S> {
    pub fn new(paths: DomainPaths, storage: S) -> Self {
        Self { paths, storage }
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

    /// Get a stored OAuth client_id for the host, or register a new one via DCR.
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

        match auth_io.read_tokens().await {
            Ok(Some(tokens)) => {
                info!(
                    "⏳ Refreshing credentials using existing tokens for {}",
                    host
                );
                match self
                    .refresh_credentials(http_client, host, &tokens.access_token)
                    .await
                {
                    Ok(creds) => {
                        info!("✔️ Successfully refreshed credentials for {}", host);
                        Ok(creds)
                    }
                    Err(e) => {
                        warn!("❌ Failed to refresh credentials for {}: {}", host, e);
                        Err(Error::Auth(
                            host.to_owned(),
                            AuthError::CredentialsRefresh(e.to_string()),
                        ))
                    }
                }
            }
            Ok(None) => {
                warn!("❌ No tokens found for {}, login required", host);
                Err(crate::Error::LoginRequired(Some(host.to_owned())))
            }
            Err(e) => {
                error!("❌ Failed to read tokens for {}: {}", host, e);
                Err(Error::Auth(
                    host.to_owned(),
                    AuthError::TokensRead(e.to_string()),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use reqwest::header::HeaderMap;
    use test_log::test;

    use crate::io::storage::mocks::MockStorage;
    use crate::paths::DomainPaths;

    const ACCESS_TOKEN: &str = "test-access-token";
    const REFRESH_TOKEN: &str = "test-refresh-token";
    const TIMESTAMP: i64 = 1708444800;

    fn get_host() -> Host {
        "test.quilt.dev".parse().unwrap()
    }

    fn get_registry() -> String {
        "registry-test.quilt.dev".to_string()
    }

    struct TestHttpClient;

    #[async_trait]
    impl HttpClient for TestHttpClient {
        async fn get<T: serde::de::DeserializeOwned>(
            &self,
            url: &str,
            auth_token: Option<&str>,
        ) -> Res<T> {
            let registry = get_registry();

            match url {
                u if u == format!("https://{}/config.json", get_host()) => {
                    let config = QuiltStackConfig {
                        registry_url: format!("https://{registry}").parse()?,
                    };
                    Ok(serde_json::from_value(serde_json::to_value(config)?)?)
                }
                u if u == format!("https://{registry}/api/auth/get_credentials") => {
                    assert_eq!(auth_token, Some(ACCESS_TOKEN));
                    let creds = RemoteCredentials {
                        access_key_id: "test-access-key".to_string(),
                        secret_access_key: "test-secret-key".to_string(),
                        session_token: "test-session-token".to_string(),
                        expiration: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
                    };
                    Ok(serde_json::from_value(serde_json::to_value(creds)?)?)
                }
                _ => panic!("Unexpected URL: {url}"),
            }
        }

        async fn head(&self, _url: &str) -> Res<HeaderMap> {
            unimplemented!("head is not used in this test")
        }

        async fn post<T: serde::de::DeserializeOwned>(
            &self,
            url: &str,
            form_data: &HashMap<String, String>,
        ) -> Res<T> {
            assert_eq!(url, format!("https://{}/api/token", get_registry()));

            // Verify form data contains the refresh token
            assert_eq!(form_data.get("refresh_token").unwrap(), REFRESH_TOKEN);

            let tokens = RemoteTokens {
                access_token: ACCESS_TOKEN.to_string(),
                refresh_token: "new-refresh-token".to_string(),
                expires_at: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
            };
            Ok(serde_json::from_value(serde_json::to_value(tokens)?)?)
        }

        async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize + Send + Sync>(
            &self,
            _url: &str,
            _body: &B,
        ) -> Res<T> {
            unimplemented!("post_json is not used in this test")
        }
    }

    #[test(tokio::test)]
    async fn test_get_registry_url() {
        let client = TestHttpClient;
        let result = get_registry_url(&client, &get_host()).await.unwrap();
        assert_eq!(
            result,
            url::Host::Domain("registry-test.quilt.dev".to_string())
        );
    }

    #[test(tokio::test)]
    async fn test_get_auth_tokens() {
        let client = TestHttpClient;
        let tokens = get_auth_tokens(&client, &get_host(), REFRESH_TOKEN)
            .await
            .unwrap();
        assert_eq!(tokens.access_token, ACCESS_TOKEN);
        assert_eq!(tokens.refresh_token, "new-refresh-token");
        assert_eq!(
            tokens.expires_at,
            chrono::DateTime::from_timestamp(1708444800, 0).unwrap()
        );
    }

    #[test(tokio::test)]
    async fn test_refresh_credentials() {
        let client = TestHttpClient;
        let credentials = refresh_credentials(&client, &get_host(), ACCESS_TOKEN)
            .await
            .unwrap();
        assert_eq!(credentials.access_key, "test-access-key");
        assert_eq!(credentials.secret_key, "test-secret-key");
        assert_eq!(credentials.token, "test-session-token");
        assert_eq!(
            credentials.expires_at,
            chrono::DateTime::from_timestamp(1708444800, 0).unwrap()
        );
    }

    #[test(tokio::test)]
    async fn test_auth_refresh_credentials() -> Res {
        let storage = MockStorage::default();
        let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
        let auth = Auth::new(paths.clone(), storage);
        let host = get_host();

        let credentials = auth
            .refresh_credentials(&TestHttpClient, &host, ACCESS_TOKEN)
            .await?;

        // Verify returned credentials
        assert_eq!(credentials.access_key, "test-access-key");
        assert_eq!(credentials.secret_key, "test-secret-key");
        assert_eq!(credentials.token, "test-session-token");
        assert_eq!(
            credentials.expires_at,
            chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap()
        );

        // TODO: try using Rc<Storage> for every struct that owns a Storage
        // Verify credentials were written correctly
        // let auth_io = AuthIo::new(storage, paths.auth_host(&host));
        // let read_creds = auth_io.read_credentials().await?.unwrap();
        // assert_eq!(read_creds.access_key, credentials.access_key);
        // assert_eq!(read_creds.secret_key, credentials.secret_key);
        // assert_eq!(read_creds.token, credentials.token);
        // assert_eq!(read_creds.expires_at, credentials.expires_at);

        Ok(())
    }

    #[test]
    fn test_remote_credentials_deserialization() {
        // Test valid RFC3339 date
        let valid_json = r#"{
            "AccessKeyId": "test-key",
            "Expiration": "2024-02-20T15:00:00Z",
            "SecretAccessKey": "test-secret",
            "SessionToken": "test-token"
        }"#;

        let creds: RemoteCredentials = serde_json::from_str(valid_json).unwrap();
        assert_eq!(creds.access_key_id, "test-key");
        assert_eq!(creds.secret_access_key, "test-secret");
        assert_eq!(creds.session_token, "test-token");
        assert_eq!(
            creds.expiration,
            chrono::DateTime::parse_from_rfc3339("2024-02-20T15:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        );

        // Test invalid RFC3339 date
        let invalid_json = r#"{
            "AccessKeyId": "test-key",
            "Expiration": "2024-02-20 15:00:00",
            "SecretAccessKey": "test-secret",
            "SessionToken": "test-token"
        }"#;

        let error = serde_json::from_str::<RemoteCredentials>(invalid_json).unwrap_err();
        assert!(error.to_string().contains("Invalid RFC3339 date"));
    }

    const AUTH_CODE: &str = "test-auth-code";
    const CODE_VERIFIER: &str = "test-code-verifier-that-is-at-least-43-characters-long";
    const CLIENT_ID: &str = "test-client-id";
    const REDIRECT_URI: &str = "quilt://auth/callback?host=test.quilt.dev";

    struct OAuthTestHttpClient;

    #[async_trait]
    impl HttpClient for OAuthTestHttpClient {
        async fn get<T: serde::de::DeserializeOwned>(
            &self,
            url: &str,
            auth_token: Option<&str>,
        ) -> Res<T> {
            let registry = get_registry();

            match url {
                u if u == format!("https://{}/config.json", get_host()) => {
                    let config = QuiltStackConfig {
                        registry_url: format!("https://{registry}").parse()?,
                    };
                    Ok(serde_json::from_value(serde_json::to_value(config)?)?)
                }
                u if u == format!("https://{registry}/api/auth/get_credentials") => {
                    assert_eq!(auth_token, Some(ACCESS_TOKEN));
                    let creds = RemoteCredentials {
                        access_key_id: "oauth-access-key".to_string(),
                        secret_access_key: "oauth-secret-key".to_string(),
                        session_token: "oauth-session-token".to_string(),
                        expiration: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
                    };
                    Ok(serde_json::from_value(serde_json::to_value(creds)?)?)
                }
                _ => panic!("Unexpected GET URL: {url}"),
            }
        }

        async fn head(&self, _url: &str) -> Res<HeaderMap> {
            unimplemented!()
        }

        async fn post<T: serde::de::DeserializeOwned>(
            &self,
            url: &str,
            form_data: &HashMap<String, String>,
        ) -> Res<T> {
            assert_eq!(url, connect_token_url(&get_host()));
            assert_eq!(form_data.get("grant_type").unwrap(), "authorization_code");
            assert_eq!(form_data.get("code").unwrap(), AUTH_CODE);
            assert_eq!(form_data.get("code_verifier").unwrap(), CODE_VERIFIER);
            assert_eq!(form_data.get("redirect_uri").unwrap(), REDIRECT_URI);
            assert_eq!(form_data.get("client_id").unwrap(), CLIENT_ID);

            let tokens = OAuthTokenResponse {
                access_token: ACCESS_TOKEN.to_string(),
                refresh_token: "oauth-refresh-token".to_string(),
                expires_in: 3600,
            };
            Ok(serde_json::from_value(serde_json::to_value(&tokens)?)?)
        }

        async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize + Send + Sync>(
            &self,
            url: &str,
            _body: &B,
        ) -> Res<T> {
            assert_eq!(url, connect_register_url(&get_host()));
            let response = DcrResponse {
                client_id: "test-dcr-client-id".to_string(),
            };
            Ok(serde_json::from_value(serde_json::to_value(response)?)?)
        }
    }

    #[test]
    fn test_connect_host() {
        let host: Host = "test.quilt.dev".parse().unwrap();
        assert_eq!(connect_host(&host), "test-connect.quilt.dev");
    }

    #[test]
    fn test_connect_token_url() {
        let host: Host = "test.quilt.dev".parse().unwrap();
        assert_eq!(
            connect_token_url(&host),
            "https://test-connect.quilt.dev/auth/token"
        );
    }

    #[test(tokio::test)]
    async fn test_exchange_oauth_code() {
        let client = OAuthTestHttpClient;
        let params = OAuthParams {
            code: AUTH_CODE.to_string(),
            code_verifier: CODE_VERIFIER.to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
            client_id: CLIENT_ID.to_string(),
        };
        let tokens = exchange_oauth_code(&client, &get_host(), &params)
            .await
            .unwrap();
        assert_eq!(tokens.access_token, ACCESS_TOKEN);
        assert_eq!(tokens.refresh_token, "oauth-refresh-token");
    }

    #[test]
    fn test_pkce_challenge() {
        let pkce = pkce_challenge();

        // Verifier should be 86 characters (64 bytes base64url-encoded without padding)
        assert_eq!(pkce.code_verifier.len(), 86);

        // Challenge should be 43 characters (SHA-256 is 32 bytes, base64url-encoded)
        assert_eq!(pkce.code_challenge.len(), 43);

        // Verify the challenge is the S256 hash of the verifier
        let expected_challenge =
            URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.code_verifier.as_bytes()));
        assert_eq!(pkce.code_challenge, expected_challenge);

        // Two calls should produce different verifiers
        let pkce2 = pkce_challenge();
        assert_ne!(pkce.code_verifier, pkce2.code_verifier);
    }

    #[test(tokio::test)]
    async fn test_login_oauth() -> Res {
        let storage = MockStorage::default();
        let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
        let auth = Auth::new(paths, storage);
        let host = get_host();

        let params = OAuthParams {
            code: AUTH_CODE.to_string(),
            code_verifier: CODE_VERIFIER.to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
            client_id: CLIENT_ID.to_string(),
        };

        auth.login_oauth(&OAuthTestHttpClient, &host, params)
            .await?;
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_get_or_register_client() -> Res {
        let storage = MockStorage::default();
        let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
        let auth = Auth::new(paths, storage);
        let host = get_host();

        // First call registers via DCR
        let client = auth
            .get_or_register_client(&OAuthTestHttpClient, &host, REDIRECT_URI)
            .await?;
        assert_eq!(client.client_id, "test-dcr-client-id");
        assert_eq!(client.redirect_uri, REDIRECT_URI);

        // Second call with same redirect_uri reads from storage (no DCR call)
        let client2 = auth
            .get_or_register_client(&OAuthTestHttpClient, &host, REDIRECT_URI)
            .await?;
        assert_eq!(client2.client_id, "test-dcr-client-id");

        // Third call with different redirect_uri re-registers
        let client3 = auth
            .get_or_register_client(&OAuthTestHttpClient, &host, "quilt://auth/callback?host=test.quilt.dev")
            .await?;
        assert_eq!(client3.client_id, "test-dcr-client-id");
        assert_eq!(client3.redirect_uri, "quilt://auth/callback?host=test.quilt.dev");

        Ok(())
    }
}
