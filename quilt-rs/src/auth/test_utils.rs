//! Shared constants and mock HTTP clients for the auth test modules.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

use super::oauth::OAuthTokenResponse;
use super::oauth::connect_register_url;
use super::oauth::connect_token_url;
use super::registry::QuiltStackConfig;
use super::registry::RemoteCredentials;
use super::registry::RemoteTokens;
use crate::Error;
use crate::Res;
use crate::io::remote::client::HttpClient;
use quilt_uri::Host;

pub(super) const ACCESS_TOKEN: &str = "test-access-token";
pub(super) const REFRESH_TOKEN: &str = "test-refresh-token";
pub(super) const TIMESTAMP: i64 = 1_708_444_800;

pub(super) fn get_host() -> Host {
    "test.quilt.dev".parse().unwrap()
}

pub(super) fn get_registry() -> String {
    "registry-test.quilt.dev".to_string()
}

pub(super) const AUTH_CODE: &str = "test-auth-code";
pub(super) const CODE_VERIFIER: &str = "test-code-verifier-that-is-at-least-43-characters-long";
pub(super) const CLIENT_ID: &str = "test-client-id";
pub(super) const REDIRECT_URI: &str = "quilt://auth/callback?host=test.quilt.dev";

pub(super) const REFRESHED_ACCESS_TOKEN: &str = "refreshed-access-token";

pub(super) struct TestHttpClient;

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

pub(super) struct OAuthTestHttpClient {
    /// The access token expected when hitting the credentials endpoint.
    pub(super) expected_credentials_token: &'static str,
}

impl Default for OAuthTestHttpClient {
    fn default() -> Self {
        Self {
            expected_credentials_token: ACCESS_TOKEN,
        }
    }
}

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
                assert_eq!(auth_token, Some(self.expected_credentials_token));
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

        let tokens = match form_data.get("grant_type").map(String::as_str) {
            Some("authorization_code") => {
                assert_eq!(form_data.get("code").unwrap(), AUTH_CODE);
                assert_eq!(form_data.get("code_verifier").unwrap(), CODE_VERIFIER);
                assert_eq!(form_data.get("redirect_uri").unwrap(), REDIRECT_URI);
                assert_eq!(form_data.get("client_id").unwrap(), CLIENT_ID);
                OAuthTokenResponse {
                    access_token: ACCESS_TOKEN.to_string(),
                    refresh_token: Some("oauth-refresh-token".to_string()),
                    expires_in: 3600,
                }
            }
            Some("refresh_token") => {
                assert_eq!(form_data.get("refresh_token").unwrap(), REFRESH_TOKEN);
                assert_eq!(form_data.get("client_id").unwrap(), CLIENT_ID);
                OAuthTokenResponse {
                    access_token: "refreshed-access-token".to_string(),
                    refresh_token: Some("new-refresh-token".to_string()),
                    expires_in: 3600,
                }
            }
            other => panic!("Unexpected grant_type: {other:?}"),
        };
        Ok(serde_json::from_value(serde_json::to_value(&tokens)?)?)
    }

    async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize + Send + Sync>(
        &self,
        url: &str,
        body: &B,
    ) -> Res<T> {
        assert_eq!(url, connect_register_url(&get_host()));
        let json = serde_json::to_value(body)?;
        assert_eq!(json["client_name"], "QuiltSync");
        assert_eq!(json["token_endpoint_auth_method"], "none");
        let redirect_uris = json["redirect_uris"].as_array().expect("redirect_uris");
        assert_eq!(redirect_uris.len(), 1);
        assert!(
            redirect_uris[0]
                .as_str()
                .unwrap()
                .starts_with("quilt://auth/callback?host=")
        );
        Ok(serde_json::from_value(serde_json::json!({
            "client_id": "test-dcr-client-id"
        }))?)
    }
}

/// Spawns a one-connection TCP responder that replies with `response` bytes.
/// Used to produce real `reqwest::Error` values with a chosen HTTP status.
pub(super) async fn spawn_one_shot(response: Vec<u8>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

/// Produce an `Error::Reqwest` whose `.status()` is the given code. There
/// is no public constructor for `reqwest::Error`, so we round-trip through
/// a real HTTP request against a canned local responder.
pub(super) async fn reqwest_error_with_status(status: u16) -> Error {
    let body = format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .into_bytes();
    let addr = spawn_one_shot(body).await;
    reqwest::Client::new()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err()
        .into()
}
