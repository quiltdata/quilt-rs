use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::de::DeserializeOwned;

use crate::Res;

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn get<T: DeserializeOwned>(&self, url: &str, auth_token: Option<&str>) -> Res<T>;
    async fn head(&self, url: &str) -> Res<HeaderMap>;
    async fn post<T: DeserializeOwned>(
        &self,
        url: &str,
        form_data: &HashMap<String, String>,
    ) -> Res<T>;
}

#[derive(Clone, Debug)]
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

const USER_AGENT: &str =
    "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.1; SV1; .NET CLR 1.0.3705; .NET CLR 1.1.4322)";

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get<T: DeserializeOwned>(&self, url: &str, auth_token: Option<&str>) -> Res<T> {
        let mut request = self.client.get(url).header("User-Agent", USER_AGENT);

        if let Some(token) = auth_token {
            request = request.bearer_auth(token);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(response.error_for_status().unwrap_err().into());
        }
        Ok(response.json().await?)
    }

    async fn head(&self, url: &str) -> Res<HeaderMap> {
        let response = self
            .client
            .head(url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await?;
        Ok(response.headers().clone())
    }

    async fn post<T: DeserializeOwned>(
        &self,
        url: &str,
        form_data: &HashMap<String, String>,
    ) -> Res<T> {
        let response = self
            .client
            .post(url)
            .header("User-Agent", USER_AGENT)
            .form(form_data)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(response.error_for_status().unwrap_err().into());
        }
        Ok(response.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_config() {
        // Skip this test in CI environments where network access might be limited
        if std::env::var("CI").is_ok() {
            return;
        }

        let client = ReqwestClient::new();
        
        // Get the raw text content first to check for the QUILT_CATALOG_CONFIG string
        let response = reqwest::get("https://open.quilt.bio/config.js")
            .await
            .expect("Failed to fetch config.js");
        let text = response.text().await.expect("Failed to get response text");
        
        // Check that the config.js contains the QUILT_CATALOG_CONFIG string
        assert!(text.contains("QUILT_CATALOG_CONFIG"), 
                "Config.js should contain QUILT_CATALOG_CONFIG string");
        
        // Now test the actual HttpClient.get method with JSON parsing
        let result = client.get::<serde_json::Value>("https://open.quilt.bio/config.json", None).await;
        
        // Verify that we can successfully make the request and parse the JSON
        assert!(result.is_ok(), "Failed to get config: {:?}", result.err());
        
        // Verify that the response contains expected fields
        let config = result.unwrap();
        assert!(config.is_object(), "Expected JSON object response");
        
        // The config should have a registry_url field
        assert!(config.get("registry_url").is_some(), "Missing registry_url field in config");
    }
}
