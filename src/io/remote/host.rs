//! Host configuration functionality
//!
//! This module handles fetching and parsing host configuration from remote endpoints.

use serde::Deserialize;

use crate::io::remote::client::HttpClient;
use crate::Error;
use crate::Res;

/// Supported checksum algorithms for a host
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostChecksums {
    /// CRC64 checksums (NVMe variant)
    Crc64,
    /// SHA256 checksums
    // Sha256, Legacy, we dont' use it
    /// SHA256 chunked checksums
    Sha256Chunked,
}

/// Configuration returned by a host
#[derive(Clone, Debug, PartialEq)]
pub struct HostConfig {
    /// Supported checksum algorithms
    pub checksums: HostChecksums,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            checksums: HostChecksums::Sha256Chunked,
        }
    }
}

/// JSON response structure from host config endpoint
#[derive(Deserialize)]
struct ConfigResponse {
    #[serde(rename = "crc64Checksums")]
    crc64_checksums: Option<bool>,
}

/// Fetch host configuration from the given host
///
/// Makes a request to `https://${host}/config.json` and parses the response
/// to determine the supported checksum algorithms.
///
/// # Arguments
/// * `client` - HTTP client implementation to use for the request
/// * `host` - Host name (e.g. "open.quiltdata.com")
///
/// # Returns
/// * `Ok(HostConfig)` - Successfully parsed host configuration
/// * `Err(Error::HostConfig)` - Failed to fetch or parse configuration
/// * `Err(Error::Reqwest)` - HTTP request failed
/// * `Err(Error::Json)` - JSON parsing failed
pub async fn fetch_host_config(client: &impl HttpClient, host: &str) -> Res<HostConfig> {
    let url = format!("https://{}/config.json", host);

    let response: ConfigResponse = client
        .get(&url, None)
        .await
        .map_err(|e| Error::HostConfig(format!("Failed to fetch config from {}: {}", host, e)))?;

    // Determine checksum algorithm based on crc64Checksums field
    let checksums = match response.crc64_checksums {
        Some(true) => HostChecksums::Crc64,
        Some(false) | None => HostChecksums::Sha256Chunked, // Default
    };

    Ok(HostConfig { checksums })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use reqwest::header::HeaderMap;
    use serde::de::DeserializeOwned;
    use std::collections::HashMap;

    // Mock HTTP client for testing
    struct MockHttpClient {
        responses: std::collections::HashMap<String, Result<String, String>>,
    }

    impl MockHttpClient {
        fn new() -> Self {
            Self {
                responses: HashMap::new(),
            }
        }

        fn add_response(&mut self, url: String, response: Result<String, String>) {
            self.responses.insert(url, response);
        }
    }

    #[async_trait]
    impl HttpClient for MockHttpClient {
        async fn get<T: DeserializeOwned>(&self, url: &str, _auth_token: Option<&str>) -> Res<T> {
            match self.responses.get(url) {
                Some(Ok(response_body)) => {
                    let response: T = serde_json::from_str(response_body)?;
                    Ok(response)
                }
                Some(Err(error)) => Err(Error::HostConfig(error.clone())),
                None => Err(Error::HostConfig(format!(
                    "No mock response for URL: {}",
                    url
                ))),
            }
        }

        async fn head(&self, _url: &str) -> Res<HeaderMap> {
            unimplemented!("head not needed for host config tests")
        }

        async fn post<T: DeserializeOwned>(
            &self,
            _url: &str,
            _form_data: &HashMap<String, String>,
        ) -> Res<T> {
            unimplemented!("post not needed for host config tests")
        }
    }

    #[tokio::test]
    async fn test_fetch_host_config_crc64_enabled() -> Res<()> {
        let mut client = MockHttpClient::new();
        client.add_response(
            "https://example.com/config.json".to_string(),
            Ok(r#"{"crc64Checksums": true}"#.to_string()),
        );

        let config = fetch_host_config(&client, "example.com").await?;
        assert_eq!(config.checksums, HostChecksums::Crc64);

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_host_config_crc64_disabled() -> Res<()> {
        let mut client = MockHttpClient::new();
        client.add_response(
            "https://example.com/config.json".to_string(),
            Ok(r#"{"crc64Checksums": false}"#.to_string()),
        );

        let config = fetch_host_config(&client, "example.com").await?;
        assert_eq!(config.checksums, HostChecksums::Sha256Chunked);

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_host_config_crc64_missing() -> Res<()> {
        let mut client = MockHttpClient::new();
        client.add_response(
            "https://example.com/config.json".to_string(),
            Ok(r#"{}"#.to_string()),
        );

        let config = fetch_host_config(&client, "example.com").await?;
        assert_eq!(config.checksums, HostChecksums::Sha256Chunked);

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_host_config_other_fields_ignored() -> Res<()> {
        let mut client = MockHttpClient::new();
        client.add_response(
            "https://example.com/config.json".to_string(),
            Ok(r#"{"crc64Checksums": true, "mode": "OPEN", "other": "ignored"}"#.to_string()),
        );

        let config = fetch_host_config(&client, "example.com").await?;
        assert_eq!(config.checksums, HostChecksums::Crc64);

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_host_config_network_error() {
        let mut client = MockHttpClient::new();
        client.add_response(
            "https://example.com/config.json".to_string(),
            Err("Network error".to_string()),
        );

        let result = fetch_host_config(&client, "example.com").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Network error"));
    }

    #[tokio::test]
    async fn test_fetch_host_config_invalid_json() {
        let mut client = MockHttpClient::new();
        client.add_response(
            "https://example.com/config.json".to_string(),
            Ok(r#"invalid json"#.to_string()),
        );

        let result = fetch_host_config(&client, "example.com").await;
        assert!(result.is_err());

        // JSON parsing errors get wrapped in HostConfig error by map_err
        let error = result.unwrap_err();
        match error {
            Error::HostConfig(msg) if msg.contains("JSON error") => {
                // This is expected - all client errors get wrapped
            }
            _ => panic!(
                "Expected HostConfig error wrapping JSON error, got: {:?}",
                error
            ),
        }
    }
}
