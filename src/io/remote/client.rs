use reqwest::{RequestBuilder, header::HeaderMap};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use crate::Res;

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn get<T: DeserializeOwned>(&self, url: &str, auth_token: Option<&str>) -> Res<T>;
    async fn head(&self, url: &str) -> Res<HeaderMap>;
    fn post(&self, url: &str) -> RequestBuilder;
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

const USER_AGENT: &str =
    "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.1; SV1; .NET CLR 1.0.3705; .NET CLR 1.1.4322)";

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get<T: DeserializeOwned>(&self, url: &str, auth_token: Option<&str>) -> Res<T> {
        let mut request = self.client
            .get(url)
            .header("User-Agent", USER_AGENT);
            
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

    fn post(&self, url: &str) -> RequestBuilder {
        self.client.post(url)
    }

    fn json<T: serde::Serialize + Send + Sync>(&self, json: &T) -> RequestBuilder {
        self.client.get("").json(json)
    }

    fn form<T: serde::Serialize + Send + Sync>(&self, form: &T) -> RequestBuilder {
        self.client.get("").form(form)
    }
}
