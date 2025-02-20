use reqwest::{RequestBuilder, header::HeaderMap};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use crate::Res;

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn get<T: DeserializeOwned>(&self, url: &str) -> Res<T>;
    async fn head(&self, url: &str) -> Res<HeaderMap>;
    fn post(&self, url: &str) -> RequestBuilder;
    fn bearer_auth(&self, token: &str) -> RequestBuilder;
    fn json<T: serde::Serialize + Send + Sync>(&self, json: &T) -> RequestBuilder;
    fn form<T: serde::Serialize + Send + Sync>(&self, form: &T) -> RequestBuilder;
}

#[derive(Clone, Debug)]
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get<T: DeserializeOwned>(&self, url: &str) -> Res<T> {
        let response = self.client.get(url).send().await?;
        Ok(response.json().await?)
    }

    async fn head(&self, url: &str) -> Res<HeaderMap> {
        let response = self.client.head(url).send().await?;
        Ok(response.headers().clone())
    }

    fn post(&self, url: &str) -> RequestBuilder {
        self.client.post(url)
    }

    fn bearer_auth(&self, token: &str) -> RequestBuilder {
        self.client.get("").bearer_auth(token)
    }

    fn json<T: serde::Serialize + Send + Sync>(&self, json: &T) -> RequestBuilder {
        self.client.get("").json(json)
    }

    fn form<T: serde::Serialize + Send + Sync>(&self, form: &T) -> RequestBuilder {
        self.client.get("").form(form)
    }
}
