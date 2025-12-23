use std::collections::HashMap;

use crate::io::remote::client::HttpClient;
use chrono::serde::ts_seconds;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

use crate::error::AuthError;
use crate::io::storage::auth::AuthIo;
use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::Tokens;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::paths::DomainPaths;
use crate::uri::Host;
use crate::Error;
use crate::Res;

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

impl<S: Storage + Clone> Auth<S> {
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
}
