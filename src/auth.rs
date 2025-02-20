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

    struct TestHttpClient;

    #[async_trait]
    impl HttpClient for TestHttpClient {
        async fn get<T: serde::de::DeserializeOwned>(
            &self,
            url: &str,
            _auth_token: Option<&str>,
        ) -> Res<T> {
            // This test is only for the default Host
            let host = Host::default();
            assert_eq!(url, format!("https://{}/config.json", host));

            let config = QuiltStackConfig {
                registry_url: format!("https://registry-{}", host).parse()?,
            };
            Ok(serde_json::from_value(serde_json::to_value(config)?)?)
        }

        async fn head(&self, _url: &str) -> Res<HeaderMap> {
            unimplemented!("head is not used in this test")
        }

        async fn post<T: serde::de::DeserializeOwned, F: serde::Serialize + Send + Sync>(
            &self,
            url: &str,
            form_data: &F,
        ) -> Res<T> {
            // This test is only for the default Host
            let host = Host::default();
            assert_eq!(url, format!("https://registry-{}/api/token", host));

            // Verify form data contains the refresh token
            let form_map: &HashMap<String, String> = form_data.downcast_ref().unwrap();
            assert_eq!(form_map.get("refresh_token").unwrap(), "test-refresh-token");

            let tokens = RemoteTokens {
                access_token: "test-access-token".to_string(),
                refresh_token: "new-refresh-token".to_string(),
                expires_at: chrono::DateTime::from_timestamp(1708444800, 0).unwrap(),
            };
            Ok(serde_json::from_value(serde_json::to_value(tokens)?)?)
        }
    }

    #[tokio::test]
    async fn test_get_registry_url() {
        let client = TestHttpClient;
        let host = Host::default();
        let result = get_registry_url(&client, &host).await.unwrap();
        assert_eq!(
            result,
            url::Host::Domain("registry-test.quilt.dev".to_string())
        );
    }

    #[tokio::test]
    async fn test_get_auth_tokens() {
        let client = TestHttpClient;
        let host = Host::default();
        let refresh_token = "test-refresh-token";

        let tokens = get_auth_tokens(&client, &host, refresh_token).await.unwrap();
        assert_eq!(tokens.access_token, "test-access-token");
        assert_eq!(tokens.refresh_token, "new-refresh-token");
        assert_eq!(
            tokens.expires_at,
            chrono::DateTime::from_timestamp(1708444800, 0).unwrap()
        );
    }
}
