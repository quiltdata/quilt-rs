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
    async fn post_json<T: DeserializeOwned, B: serde::Serialize + Send + Sync>(
        &self,
        url: &str,
        body: &B,
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

    async fn post_json<T: DeserializeOwned, B: serde::Serialize + Send + Sync>(
        &self,
        url: &str,
        body: &B,
    ) -> Res<T> {
        let response = self
            .client
            .post(url)
            .header("User-Agent", USER_AGENT)
            .json(body)
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
    use test_log::test;

    use serde::Deserialize;
    use serde::Serialize;

    #[test(tokio::test)]
    async fn test_get_config() -> Res {
        let client = ReqwestClient::new();

        #[derive(Deserialize, Serialize)]
        struct Config {
            mode: String,
        }

        // Get the raw text content first to check for the QUILT_CATALOG_CONFIG string
        let response: Config = client
            .get("https://open.quiltdata.com/config.json", None)
            .await?;

        // Check that the config.js contains the QUILT_CATALOG_CONFIG string
        assert_eq!(response.mode, "OPEN");

        Ok(())
    }
}
