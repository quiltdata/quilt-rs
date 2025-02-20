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
        Ok(response.json().await?)
    }

    async fn head(&self, url: &str) -> Res<HeaderMap> {
        let response = self.client.head(url).send().await?;
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
        Ok(response.json().await?)
    }
}
