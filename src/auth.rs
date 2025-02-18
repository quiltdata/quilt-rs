use std::collections::HashMap;

use chrono::serde::ts_seconds;
use reqwest::Client as HttpClient;
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

const USER_AGENT: &str =
    "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.1; SV1; .NET CLR 1.0.3705; .NET CLR 1.1.4322)";

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

async fn get_registry_url(http_client: &HttpClient, host: &Host) -> Res<url::Host> {
    let request = http_client
        .get(format!("https://{}/config.json", host))
        .header("User-Agent", USER_AGENT)
        .build()?;
    let response = http_client.execute(request).await?;

    let QuiltStackConfig { registry_url } = response.json().await?;
    Ok(url::Host::Domain(
        registry_url
            .domain()
            .ok_or(crate::Error::LoginRequiredRegistryUrl(host.clone()))?
            .to_string(),
    ))
}

async fn get_auth_tokens(
    http_client: &HttpClient,
    host: &Host,
    refresh_token: &str,
) -> Res<Tokens> {
    let registry = get_registry_url(http_client, host).await?;

    let mut form_data: HashMap<String, String> = HashMap::new();
    form_data.insert("refresh_token".to_string(), refresh_token.to_string());
    let request = http_client
        .post(format!("https://{}/api/token", registry))
        .header("User-Agent", USER_AGENT)
        .form(&form_data)
        .build()?;
    let response = http_client.execute(request).await?;

    let tokens_json: RemoteTokens = response.json().await?;
    let tokens = Tokens::from(tokens_json);

    Ok(tokens)
}

async fn refresh_credentials(
    http_client: &HttpClient,
    host: &Host,
    access_token: &str,
) -> Res<Credentials> {
    let registry = get_registry_url(http_client, host).await?;

    let empty: HashMap<String, String> = HashMap::new();
    let request = http_client
        .get(format!("https://{}/api/auth/get_credentials", registry))
        .bearer_auth(access_token)
        .header("User-Agent", USER_AGENT)
        .json(&empty)
        .build()?;
    let response = http_client.execute(request).await?;

    let creds_json: RemoteCredentials = response.json().await?;
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

    pub async fn login(&self, http_client: &HttpClient, host: &Host, refresh_token: String) -> Res {
        let tokens = self
            .get_auth_tokens(http_client, host, &refresh_token)
            .await?;

        self.save_tokens(host, &tokens).await?;

        self.refresh_credentials(http_client, host, &tokens.access_token)
            .await?;

        Ok(())
    }

    async fn get_auth_tokens(
        &self,
        http_client: &HttpClient,
        host: &Host,
        refresh_token: &str,
    ) -> Res<Tokens> {
        get_auth_tokens(http_client, host, refresh_token).await
    }

    async fn save_tokens(&self, host: &Host, tokens: &Tokens) -> Res<()> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_tokens(tokens).await
    }

    async fn refresh_credentials(
        &self,
        http_client: &HttpClient,
        host: &Host,
        access_token: &str,
    ) -> Res<Credentials> {
        let credentials = refresh_credentials(http_client, host, access_token).await?;

        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        auth_io.write_credentials(&credentials).await?;

        Ok(credentials)
    }

    pub async fn get_credentials_or_refresh(
        &self,
        http_client: &HttpClient,
        host: &Host,
    ) -> Res<Credentials> {
        let auth_io = AuthIo::new(self.storage.clone(), self.paths.auth_host(host));
        match auth_io.get_credentials().await? {
            Some(creds) => Ok(creds),
            None => match auth_io.read_tokens().await? {
                Some(tokens) => {
                    self.refresh_credentials(http_client, host, &tokens.access_token)
                        .await
                }
                None => Err(crate::Error::LoginRequired),
            },
        }
    }
}
