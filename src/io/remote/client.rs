use reqwest::RequestBuilder;
use async_trait::async_trait;
use crate::Res;

#[async_trait]
pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str) -> RequestBuilder;
    fn post(&self, url: &str) -> RequestBuilder;
    async fn execute(&self, request: RequestBuilder) -> Res<reqwest::Response>;
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
    fn get(&self, url: &str) -> RequestBuilder {
        self.client.get(url)
    }

    fn post(&self, url: &str) -> RequestBuilder {
        self.client.post(url)
    }

    async fn execute(&self, request: RequestBuilder) -> Res<reqwest::Response> {
        Ok(request.send().await?)
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
