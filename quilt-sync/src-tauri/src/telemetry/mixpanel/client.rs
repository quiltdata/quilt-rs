//! The one place that knows which Mixpanel client library sends our events.
//!
//! Everything else in telemetry uses [`WireEvent`], [`SendError`] and
//! [`MixpanelClient`]. The tests in `client/tests.rs` pin the HTTP wire through
//! this API alone, so they hold for whatever sits behind it.

use std::collections::HashMap;

use mixpanel_rs::{Config, Event, Mixpanel, error::Error as Mp};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
mod tests;

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
///
/// The message is the client's own description of the failure.
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

impl From<Mp> for SendError {
    fn from(err: Mp) -> Self {
        let message = err.to_string();
        match err {
            // `ApiClientError` is also what a 2xx answer with a status other
            // than 1 becomes.
            Mp::ApiClientError(..)
            | Mp::ApiUnexpectedResponse(_)
            | Mp::ApiPayloadTooLarge
            | Mp::JsonError(_)
            | Mp::UrlError(_)
            | Mp::TimeError => Self::Refused(message),

            Mp::HttpError(_)
            | Mp::ApiServerError(_)
            | Mp::ApiRateLimitError(_)
            | Mp::ApiHttpError(..)
            | Mp::MaxRetriesReached(_) => Self::Unreachable(message),
        }
    }
}

/// Sends batches of events to the Mixpanel ingest API.
pub struct MixpanelClient {
    inner: Mixpanel,
}

impl MixpanelClient {
    pub fn new(token: &str, secret: Option<&str>) -> Self {
        Self::with_config(token, secret, Config::default())
    }

    /// A client that sends to `base_url` (`http://host:port`) instead of
    /// the real API.
    #[cfg(test)]
    pub fn with_base_url(token: &str, secret: Option<&str>, base_url: &str) -> Self {
        let (protocol, host) = base_url.split_once("://").expect("a scheme://host URL");
        let config = Config {
            protocol: protocol.to_owned(),
            host: host.trim_end_matches('/').to_owned(),
            ..Config::default()
        };
        Self::with_config(token, secret, config)
    }

    fn with_config(token: &str, secret: Option<&str>, config: Config) -> Self {
        let config = Config {
            secret: secret.map(ToOwned::to_owned),
            // Without this the API does not say whether it accepted an event,
            // so a rejected event returns HTTP 200 and reads as a success.
            verbose: true,
            ..config
        };
        Self {
            inner: Mixpanel::init(token, Some(config)),
        }
    }

    /// Send `events`, in order, retrying what the API may accept later.
    ///
    /// Stops at the first request that fails; requests before it stay
    /// delivered.
    pub async fn send(&self, events: Vec<WireEvent>) -> Result<(), SendError> {
        let events = events
            .into_iter()
            .map(|WireEvent { event, properties }| Event { event, properties })
            .collect();
        Ok(self.inner.track_batch(events).await?)
    }
}
