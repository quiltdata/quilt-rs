//! Registry API calls and wire types: token refresh via the legacy
//! `/api/token` endpoint and S3 credential vending.

use std::collections::HashMap;
use std::fmt;

use chrono::serde::ts_seconds;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::Res;
use crate::error::LoginError;
use crate::io::remote::client::HttpClient;
use crate::io::storage::auth::Credentials;
use crate::io::storage::auth::Tokens;
use quilt_uri::Host;

#[derive(Deserialize, Serialize)]
pub struct RemoteTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(with = "ts_seconds")]
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl fmt::Debug for RemoteTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteTokens")
            .field("expires_at", &self.expires_at)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
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

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct RemoteCredentials {
    pub(super) access_key_id: String,
    #[serde(deserialize_with = "date_from_rfc3339")]
    pub(super) expiration: chrono::DateTime<chrono::Utc>,
    pub(super) secret_access_key: String,
    pub(super) session_token: String,
}

impl fmt::Debug for RemoteCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteCredentials")
            .field("expiration", &self.expiration)
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field("session_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
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
pub(super) struct QuiltStackConfig {
    pub(super) registry_url: url::Url,
}

pub(super) async fn get_registry_url(http_client: &impl HttpClient, host: &Host) -> Res<url::Host> {
    let QuiltStackConfig { registry_url } = http_client
        .get(&format!("https://{host}/config.json"), None)
        .await?;
    Ok(url::Host::Domain(
        registry_url
            .domain()
            .ok_or(LoginError::RequiredRegistryUrl(host.to_owned()))?
            .to_string(),
    ))
}

pub(super) async fn get_auth_tokens(
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

pub(super) async fn refresh_credentials(
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

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::auth::test_utils::*;

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
            chrono::DateTime::from_timestamp(1_708_444_800, 0).unwrap()
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
            chrono::DateTime::from_timestamp(1_708_444_800, 0).unwrap()
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

    #[test]
    fn remote_tokens_debug_redacts_secrets() {
        let tokens = RemoteTokens {
            access_token: "secret-access".to_string(),
            refresh_token: "secret-refresh".to_string(),
            expires_at: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
        };
        let output = format!("{tokens:?}");
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret-access"));
        assert!(!output.contains("secret-refresh"));
    }

    #[test]
    fn remote_credentials_debug_redacts_secrets() {
        let creds = RemoteCredentials {
            access_key_id: "secret-key-id".to_string(),
            expiration: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
            secret_access_key: "secret-access-key".to_string(),
            session_token: "secret-session-token".to_string(),
        };
        let output = format!("{creds:?}");
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret-key-id"));
        assert!(!output.contains("secret-access-key"));
        assert!(!output.contains("secret-session-token"));
    }
}
