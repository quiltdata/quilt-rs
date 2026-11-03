//! Our client for the Mixpanel ingest API.
//!
//! Everything else in telemetry uses [`WireEvent`], [`SendError`] and
//! [`MixpanelClient`]. The tests in `client/tests.rs` pin the HTTP wire.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

const API: &str = "https://api.mixpanel.com";
/// The API takes at most this many events in one request.
const CHUNK: usize = 50;
const MAX_RETRIES: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// How long one send may take, retries included. The caller bounds the send
/// with it; the client gives up early on a wait that would outlast it.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
// Library properties every event carries; changing them splits reports in
// Mixpanel.
const LIB: &str = "quiltsync";
const LIB_VERSION: &str = env!("CARGO_PKG_VERSION");

/// An event as the ingest API receives it: a name and its properties.
///
/// Also the on-disk form of each line in the offline buffer, so its JSON must
/// keep the shape `{"event": …, "properties": {…}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireEvent {
    pub event: String,
    pub properties: HashMap<String, Value>,
}

/// Why a send failed, reduced to what the caller acts on.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    /// The ingest API answered and refused the events.
    #[error("{0}")]
    Refused(String),
    /// No verdict came back: the network failed, or the API stayed unwell
    /// through every retry.
    #[error("{0}")]
    Unreachable(String),
}

/// How one request failed.
enum Failure {
    /// Final: no retry.
    Done(SendError),
    /// Worth another try, after `wait` if the API named one.
    Retry {
        message: String,
        wait: Option<Duration>,
    },
}

/// Sends batches of events to the Mixpanel ingest API.
pub struct MixpanelClient {
    http: reqwest::Client,
    token: String,
    authorization: Option<String>,
    url: String,
    /// Until when the API asked us to wait, after a wait the client did not
    /// take. In memory only: a restart forgets it.
    quiet_until: Mutex<Option<Instant>>,
}

impl MixpanelClient {
    pub fn new(token: &str, secret: Option<&str>) -> Self {
        Self::sending_to(token, secret, API)
    }

    /// A client that sends to `base_url` (`http://host:port`) instead of
    /// the real API.
    #[cfg(test)]
    pub fn with_base_url(token: &str, secret: Option<&str>, base_url: &str) -> Self {
        Self::sending_to(token, secret, base_url)
    }

    fn sending_to(token: &str, secret: Option<&str>, base_url: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("an HTTP client"),
            token: token.to_owned(),
            authorization: secret
                .map(|secret| format!("Basic {}", STANDARD.encode(format!("{secret}:")))),
            // `verbose=1` makes the API say whether it accepted the events;
            // without it a refusal is an HTTP 200 too.
            url: format!("{}/track?ip=0&verbose=1", base_url.trim_end_matches('/')),
            quiet_until: Mutex::new(None),
        }
    }

    /// Send `events`, in order, retrying what the API may accept later.
    ///
    /// Stops at the first request that fails; requests before it stay
    /// delivered.
    pub async fn send(&self, mut events: Vec<WireEvent>) -> Result<(), SendError> {
        if let Some(until) = *self.quiet_until.lock().expect("quiet_until lock")
            && Instant::now() < until
        {
            return Err(SendError::Unreachable(
                "Mixpanel API asked to wait before the next send".to_owned(),
            ));
        }
        for event in &mut events {
            let properties = &mut event.properties;
            properties.insert("token".to_owned(), Value::from(self.token.as_str()));
            properties.insert("mp_lib".to_owned(), Value::from(LIB));
            properties.insert("$lib_version".to_owned(), Value::from(LIB_VERSION));
        }
        let deadline = Instant::now() + SEND_TIMEOUT;
        for chunk in events.chunks(CHUNK) {
            let json = serde_json::to_string(chunk)
                .map_err(|err| SendError::Refused(format!("JSON serialization error: {err}")))?;
            self.post(format!("data={}", STANDARD.encode(json)), deadline)
                .await?;
        }
        Ok(())
    }

    async fn post(&self, body: String, deadline: Instant) -> Result<(), SendError> {
        let mut retries = 0;
        loop {
            let (message, wait) = match self.post_once(body.clone()).await {
                Ok(()) => return Ok(()),
                Err(Failure::Done(err)) if retries < MAX_RETRIES => return Err(err),
                Err(Failure::Done(err)) => (err.to_string(), None),
                Err(Failure::Retry { message, wait }) => (message, wait),
            };
            let left = deadline.saturating_duration_since(Instant::now());
            // A wait the API named and we will not take holds later sends.
            if let Some(wait) = wait
                && (retries == MAX_RETRIES || wait > left)
            {
                *self.quiet_until.lock().expect("quiet_until lock") =
                    Instant::now().checked_add(wait);
            }
            // Once the retries are spent, any failure ends the send this way.
            if retries == MAX_RETRIES {
                return Err(SendError::Unreachable(format!(
                    "Max retries reached: Failed after {retries} retries. Last error: {message}"
                )));
            }
            let wait = match wait {
                // Waiting would run past the send's time limit; end now.
                Some(wait) if wait > left => {
                    return Err(SendError::Unreachable(format!(
                        "{message}; longer than the send has left"
                    )));
                }
                Some(wait) => wait,
                None => BACKOFF_BASE * 2u32.pow(retries),
            };
            tokio::time::sleep(wait).await;
            retries += 1;
        }
    }

    async fn post_once(&self, body: String) -> Result<(), Failure> {
        let mut request = self
            .http
            .post(&self.url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body);
        if let Some(authorization) = &self.authorization {
            request = request.header("Authorization", authorization);
        }

        let response = request.send().await.map_err(|err| http_failure(&err))?;

        let status = response.status();
        let code = status.as_u16();
        let refused = |message: String| Err(Failure::Done(SendError::Refused(message)));

        if status.is_success() {
            let body = response.text().await.map_err(|err| http_failure(&err))?;
            let json: Value = match serde_json::from_str(&body) {
                Ok(json) => json,
                Err(err) => return refused(format!("JSON serialization error: {err}")),
            };
            return match json.get("status").and_then(Value::as_u64) {
                Some(1) => Ok(()),
                Some(_) => match json.get("error").and_then(Value::as_str) {
                    Some(error) => {
                        refused(format!("Mixpanel API client error (HTTP {code}): {error}"))
                    }
                    None => refused(format!(
                        "Mixpanel API unexpected response: Response status was not 1: {body}"
                    )),
                },
                None => refused(format!(
                    "Mixpanel API unexpected response: Response missing status: {body}"
                )),
            };
        }

        match status {
            StatusCode::PAYLOAD_TOO_LARGE => {
                refused("Mixpanel API payload too large (HTTP 413)".to_owned())
            }
            StatusCode::TOO_MANY_REQUESTS => {
                let seconds = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok());
                Err(Failure::Retry {
                    message: format!(
                        "Mixpanel API rate limited (Retry after: {seconds:?} seconds)"
                    ),
                    wait: seconds.map(Duration::from_secs),
                })
            }
            _ if code >= 500 => Err(Failure::Retry {
                message: format!("Mixpanel API server error (HTTP {code})"),
                wait: None,
            }),
            _ => {
                let body = response.text().await.unwrap_or_else(|err| err.to_string());
                if code >= 400 {
                    refused(format!("Mixpanel API client error (HTTP {code}): {body}"))
                } else {
                    Err(Failure::Done(SendError::Unreachable(format!(
                        "Mixpanel API HTTP error (HTTP {code}): {body}"
                    ))))
                }
            }
        }
    }
}

/// A request that got no answer is retried. A request we could not build is
/// our bug; a redirect loop got answers. Neither is retried.
fn http_failure(err: &reqwest::Error) -> Failure {
    let message = format!("HTTP request error: {err}");
    if err.is_builder() {
        Failure::Done(SendError::Refused(message))
    } else if err.is_redirect() {
        Failure::Done(SendError::Unreachable(message))
    } else {
        Failure::Retry {
            message,
            wait: None,
        }
    }
}
