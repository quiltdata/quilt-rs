use std::collections::HashMap;

use crate::io::remote::client::HttpClient;
use chrono::serde::ts_seconds;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::io::storage::auth::AuthIo;
use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::Tokens;
use crate::io::storage::LocalStorage;
use crate::paths::DomainPaths;
use crate::uri::Host;
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
            .map_err(|e| Error::custom(format!("Invalid RFC3339 date: {}", e)))
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
        .get(&format!("https://{}/config.json", host), None)
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
        .post(&format!("https://{}/api/token", registry), &form_data)
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
            &format!("https://{}/api/auth/get_credentials", registry),
            Some(access_token),
        )
        .await?;

    let credentials = Credentials::from(creds_json);

    Ok(credentials)
}

#[derive(Debug, Clone)]
pub struct Auth {
    pub paths: DomainPaths,
    pub storage: LocalStorage,
}

impl Auth {
    pub fn new(paths: DomainPaths, storage: LocalStorage) -> Self {
        Self { paths, storage }
    }

    pub async fn login<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        refresh_token: String,
    ) -> Res {
        let tokens = self
            .get_auth_tokens(http_client, host, &refresh_token)
            .await?;

        self.save_tokens(host, &tokens).await?;

        self.refresh_credentials(http_client, host, &tokens.access_token)
            .await?;

        Ok(())
    }

    async fn get_auth_tokens<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        refresh_token: &str,
    ) -> Res<Tokens> {
        get_auth_tokens(http_client, host, refresh_token).await
    }

    async fn save_tokens(&self, host: &Host, tokens: &Tokens) -> Res<()> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_tokens(tokens).await
    }

    async fn refresh_credentials<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
        access_token: &str,
    ) -> Res<Credentials> {
        let credentials = refresh_credentials(http_client, host, access_token).await?;

        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_credentials(&credentials).await?;

        Ok(credentials)
    }

    pub async fn get_credentials_or_refresh<T: HttpClient>(
        &self,
        http_client: &T,
        host: &Host,
    ) -> Res<Credentials> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        match auth_io.read_credentials().await? {
            Some(creds) => Ok(creds),
            None => match auth_io.read_tokens().await? {
                Some(tokens) => {
                    self.refresh_credentials(http_client, host, &tokens.access_token)
                        .await
                }
                None => Err(crate::Error::LoginRequired(Some(host.to_owned()))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use reqwest::header::HeaderMap;

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
                        registry_url: format!("https://{}", registry).parse()?,
                    };
                    Ok(serde_json::from_value(serde_json::to_value(config)?)?)
                }
                u if u == format!("https://{}/api/auth/get_credentials", registry) => {
                    assert_eq!(auth_token, Some(ACCESS_TOKEN));
                    let creds = RemoteCredentials {
                        access_key_id: "test-access-key".to_string(),
                        secret_access_key: "test-secret-key".to_string(),
                        session_token: "test-session-token".to_string(),
                        expiration: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
                    };
                    Ok(serde_json::from_value(serde_json::to_value(creds)?)?)
                }
                _ => panic!("Unexpected URL: {}", url),
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

    #[tokio::test]
    async fn test_get_registry_url() {
        let client = TestHttpClient;
        let result = get_registry_url(&client, &get_host()).await.unwrap();
        assert_eq!(
            result,
            url::Host::Domain("registry-test.quilt.dev".to_string())
        );
    }

    #[tokio::test]
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

    #[tokio::test]
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
