use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest_middleware::ClientBuilder;
use reqwest_middleware::ClientWithMiddleware;
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::DefaultRetryableStrategy;
use reqwest_retry::RetryTransientMiddleware;
use reqwest_retry::Retryable;
use reqwest_retry::RetryableStrategy;
use serde::de::DeserializeOwned;
use tracing::warn;

use crate::Error;
use crate::Res;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_RETRIES: u32 = 2;

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
    client: ClientWithMiddleware,
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestClient {
    pub fn new() -> Self {
        let inner = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .build()
            .expect("reqwest client build should not fail with default TLS config");

        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(MAX_RETRIES);
        let retry_middleware =
            RetryTransientMiddleware::new_with_policy_and_strategy(retry_policy, LoggingStrategy);

        let client = ClientBuilder::new(inner).with(retry_middleware).build();

        Self { client }
    }
}

/// Wraps [`DefaultRetryableStrategy`] with a `warn!` on every attempt the retry
/// middleware classifies as transient. Gives us a flakiness signal in logs
/// without standing up dedicated telemetry.
///
/// Fires on the *final* attempt too — reqwest-retry asks the strategy before
/// checking whether any attempts remain, so "may retry" is honest: retry
/// happens only if the attempt count hasn't been exhausted.
struct LoggingStrategy;

impl RetryableStrategy for LoggingStrategy {
    fn handle(
        &self,
        res: &Result<reqwest::Response, reqwest_middleware::Error>,
    ) -> Option<Retryable> {
        let decision = DefaultRetryableStrategy.handle(res);
        if matches!(decision, Some(Retryable::Transient)) {
            match res {
                Ok(resp) => warn!(
                    status = resp.status().as_u16(),
                    url = %resp.url(),
                    "🔁 transient HTTP response — may retry"
                ),
                Err(e) => warn!(
                    error = %e,
                    "🔁 transient HTTP error — may retry"
                ),
            }
        }
        decision
    }
}

impl From<reqwest_middleware::Error> for Error {
    fn from(err: reqwest_middleware::Error) -> Self {
        match err {
            reqwest_middleware::Error::Reqwest(e) => Error::Reqwest(e),
            // `Middleware(anyhow::Error)` is only produced if a middleware
            // layer itself fails (not the HTTP exchange). Our only middleware
            // is the retry layer, which doesn't surface errors this way; fold
            // into `Error::Io` so callers don't need a new match arm.
            reqwest_middleware::Error::Middleware(e) => {
                Error::Io(std::io::Error::other(e.to_string()))
            }
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

    // TODO: wire through `ensure_success` so non-2xx HEAD responses surface as
    // errors instead of empty-header `Ok`.
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
