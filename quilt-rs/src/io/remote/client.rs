use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::de::DeserializeOwned;
use tracing::warn;

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

/// Max bytes of response body to include in error log lines. Enough for an
/// RFC 6749 §5.2 error payload (`{"error":"invalid_grant",...}`) or a short
/// server error page, without flooding logs with a full HTML response.
const ERROR_BODY_LOG_LIMIT: usize = 500;

/// On non-2xx responses, reads and logs the status/url/body before returning
/// the reqwest error. Keeps the response body — which `error_for_status`
/// would otherwise discard — available for diagnostics.
async fn ensure_success(response: reqwest::Response) -> Res<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let url = response.url().clone();
    // Take the error via the non-consuming variant, then consume the response
    // for its body.
    let err = response
        .error_for_status_ref()
        .expect_err("status is non-success");
    let body = response.text().await.unwrap_or_default();
    warn!(
        status = status.as_u16(),
        url = %url,
        body = %truncate_for_log(&body),
        "❌ HTTP error response"
    );
    Err(err.into())
}

fn truncate_for_log(s: &str) -> String {
    if s.len() <= ERROR_BODY_LOG_LIMIT {
        return s.to_string();
    }
    let mut end = ERROR_BODY_LOG_LIMIT;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[{} bytes total]", &s[..end], s.len())
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get<T: DeserializeOwned>(&self, url: &str, auth_token: Option<&str>) -> Res<T> {
        let mut request = self.client.get(url).header("User-Agent", USER_AGENT);

        if let Some(token) = auth_token {
            request = request.bearer_auth(token);
        }

        let response = ensure_success(request.send().await?).await?;
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
        let response = ensure_success(
            self.client
                .post(url)
                .header("User-Agent", USER_AGENT)
                .form(form_data)
                .send()
                .await?,
        )
        .await?;
        Ok(response.json().await?)
    }

    async fn post_json<T: DeserializeOwned, B: serde::Serialize + Send + Sync>(
        &self,
        url: &str,
        body: &B,
    ) -> Res<T> {
        let response = ensure_success(
            self.client
                .post(url)
                .header("User-Agent", USER_AGENT)
                .json(body)
                .send()
                .await?,
        )
        .await?;
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

    #[test]
    fn truncate_short_body_is_unchanged() {
        assert_eq!(truncate_for_log("hello"), "hello");
    }

    #[test]
    fn truncate_long_body_is_cut_with_total_length() {
        let s = "x".repeat(ERROR_BODY_LOG_LIMIT + 10);
        let got = truncate_for_log(&s);
        assert!(got.starts_with(&"x".repeat(ERROR_BODY_LOG_LIMIT)));
        assert!(got.contains(&format!("[{} bytes total]", s.len())));
    }

    // `str` slicing must land on a char boundary — a multi-byte glyph at the
    // cutoff would otherwise panic.
    #[test]
    fn truncate_never_splits_multibyte_chars() {
        // "💥" is 4 bytes; put one straddling the limit.
        let prefix = "a".repeat(ERROR_BODY_LOG_LIMIT - 2);
        let s = format!("{prefix}💥trailing");
        let got = truncate_for_log(&s);
        assert!(got.contains(&prefix));
        assert!(got.contains("bytes total"));
    }
}
