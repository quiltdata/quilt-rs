//! Tests for the `Auth` store: login flows, credential refresh with
//! bounded retry, and per-host single-flight refresh locking.

use super::*;

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use test_log::test;

use super::oauth::OAuthTokenResponse;
use super::oauth::connect_token_url;
use super::registry::QuiltStackConfig;
use super::registry::RemoteCredentials;
use super::test_utils::*;
use crate::io::storage::mocks::MockStorage;

#[test(tokio::test)]
async fn test_auth_refresh_credentials() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());
    let host = get_host();

    let credentials = auth
        .refresh_credentials(&TestHttpClient, &host, ACCESS_TOKEN)
        .await?;

    // Verify returned credentials
    assert_eq!(credentials.access_key, "test-access-key");
    assert_eq!(credentials.secret_key, "test-secret-key");
    assert_eq!(credentials.token, "test-session-token");
    assert_eq!(
        credentials.expires_at,
        chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap()
    );

    // Verify credentials were persisted. Note: read_credentials() filters
    // expired credentials, so we deserialize directly from the raw bytes.
    use crate::io::storage::StorageExt;
    let creds_path = paths.auth_host(&host).join(crate::paths::AUTH_CREDENTIALS);
    let bytes = storage.read_bytes(&creds_path).await?;
    let read_creds: Credentials = serde_json::from_slice(&bytes)?;
    assert_eq!(read_creds.access_key, credentials.access_key);
    assert_eq!(read_creds.secret_key, credentials.secret_key);
    assert_eq!(read_creds.token, credentials.token);
    assert_eq!(read_creds.expires_at, credentials.expires_at);

    Ok(())
}

#[test(tokio::test)]
async fn test_login_oauth() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths, storage);
    let host = get_host();

    let params = OAuthParams {
        code: AUTH_CODE.to_string(),
        code_verifier: CODE_VERIFIER.to_string(),
        redirect_uri: REDIRECT_URI.to_string(),
        client_id: CLIENT_ID.to_string(),
    };

    auth.login_oauth(&OAuthTestHttpClient::default(), &host, params)
        .await?;
    Ok(())
}

#[test(tokio::test)]
async fn test_get_credentials_or_refresh_with_expired_token() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());
    let host = get_host();

    // Seed an expired access token and a stored OAuth client.
    let auth_io = AuthIo::new(storage, paths.auth_host(&host));
    auth_io
        .write_tokens(&Tokens {
            access_token: "expired-access-token".to_string(),
            refresh_token: REFRESH_TOKEN.to_string(),
            expires_at: chrono::Utc::now() - chrono::Duration::seconds(300),
        })
        .await?;
    auth_io
        .write_client(&OAuthClient {
            client_id: CLIENT_ID.to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
        })
        .await?;

    let client = OAuthTestHttpClient {
        expected_credentials_token: REFRESHED_ACCESS_TOKEN,
    };
    let creds = auth.get_credentials_or_refresh(&client, &host).await?;

    // Credentials should come from the refreshed access token.
    assert_eq!(creds.access_key, "oauth-access-key");

    // Verify the new tokens were persisted by reading them back.
    let persisted = auth_io
        .read_tokens()
        .await?
        .expect("tokens should be persisted");
    assert_eq!(persisted.access_token, REFRESHED_ACCESS_TOKEN);
    assert_eq!(persisted.refresh_token, "new-refresh-token");

    Ok(())
}

#[test(tokio::test)]
async fn test_get_or_register_client() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths, storage);
    let host = get_host();

    // First call registers via DCR
    let client = auth
        .get_or_register_client(&OAuthTestHttpClient::default(), &host, REDIRECT_URI)
        .await?;
    assert_eq!(client.client_id, "test-dcr-client-id");
    assert_eq!(client.redirect_uri, REDIRECT_URI);

    // Second call with same redirect_uri reads from storage (no DCR call)
    let client2 = auth
        .get_or_register_client(&OAuthTestHttpClient::default(), &host, REDIRECT_URI)
        .await?;
    assert_eq!(client2.client_id, "test-dcr-client-id");

    // Third call with different redirect_uri re-registers
    let new_redirect = "quilt://auth/callback?host=other.quilt.dev";
    let client3 = auth
        .get_or_register_client(&OAuthTestHttpClient::default(), &host, new_redirect)
        .await?;
    assert_eq!(client3.client_id, "test-dcr-client-id");
    assert_eq!(client3.redirect_uri, new_redirect);

    Ok(())
}

/// Mock that fails the first N calls against each endpoint with a real
/// `Error::Reqwest` carrying HTTP 401, then starts succeeding.
struct RetryMockClient {
    cred_fail_first_n: usize,
    token_fail_first_n: usize,
    cred_calls: AtomicUsize,
    token_calls: AtomicUsize,
}

impl RetryMockClient {
    fn new(cred_fail: usize, token_fail: usize) -> Self {
        Self {
            cred_fail_first_n: cred_fail,
            token_fail_first_n: token_fail,
            cred_calls: AtomicUsize::new(0),
            token_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl HttpClient for RetryMockClient {
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        _auth_token: Option<&str>,
    ) -> Res<T> {
        let registry = get_registry();
        if url == format!("https://{}/config.json", get_host()) {
            let config = QuiltStackConfig {
                registry_url: format!("https://{registry}").parse()?,
            };
            return Ok(serde_json::from_value(serde_json::to_value(config)?)?);
        }
        if url == format!("https://{registry}/api/auth/get_credentials") {
            let n = self.cred_calls.fetch_add(1, Ordering::SeqCst);
            if n < self.cred_fail_first_n {
                return Err(reqwest_error_with_status(401).await);
            }
            let creds = RemoteCredentials {
                access_key_id: "oauth-access-key".to_string(),
                secret_access_key: "oauth-secret-key".to_string(),
                session_token: "oauth-session-token".to_string(),
                expiration: chrono::DateTime::from_timestamp(TIMESTAMP, 0).unwrap(),
            };
            return Ok(serde_json::from_value(serde_json::to_value(creds)?)?);
        }
        panic!("Unexpected GET URL: {url}")
    }

    async fn head(&self, _url: &str) -> Res<HeaderMap> {
        unimplemented!()
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form_data: &HashMap<String, String>,
    ) -> Res<T> {
        assert_eq!(url, connect_token_url(&get_host()));
        let n = self.token_calls.fetch_add(1, Ordering::SeqCst);
        if n < self.token_fail_first_n {
            return Err(reqwest_error_with_status(401).await);
        }
        assert_eq!(
            form_data.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        let tokens = OAuthTokenResponse {
            access_token: REFRESHED_ACCESS_TOKEN.to_string(),
            refresh_token: Some("new-refresh-token".to_string()),
            expires_in: 3600,
        };
        Ok(serde_json::from_value(serde_json::to_value(&tokens)?)?)
    }

    async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize + Send + Sync>(
        &self,
        _url: &str,
        _body: &B,
    ) -> Res<T> {
        unimplemented!()
    }
}

async fn seed_fresh_tokens(storage: &Arc<MockStorage>, paths: &DomainPaths, host: &Host) {
    let auth_io = AuthIo::new(storage.clone(), paths.auth_host(host));
    auth_io
        .write_tokens(&Tokens {
            access_token: ACCESS_TOKEN.to_string(),
            refresh_token: REFRESH_TOKEN.to_string(),
            // Well inside the 60-second buffer → proactive refresh skipped.
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();
    auth_io
        .write_client(&OAuthClient {
            client_id: CLIENT_ID.to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
        })
        .await
        .unwrap();
}

/// Credentials endpoint flaps 401 once, then succeeds. The retry path
/// force-refreshes the access token and re-hits the credentials endpoint;
/// user must not see `LoginRequired`.
#[test(tokio::test)]
async fn test_credentials_transient_401_recovers_via_force_token_refresh() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());
    let host = get_host();
    seed_fresh_tokens(&storage, &paths, &host).await;

    let client = RetryMockClient::new(/*cred_fail=*/ 1, /*token_fail=*/ 0);
    let creds = auth.get_credentials_or_refresh(&client, &host).await?;

    assert_eq!(creds.access_key, "oauth-access-key");
    assert_eq!(
        client.cred_calls.load(Ordering::SeqCst),
        2,
        "credentials endpoint should be called twice: initial + retry"
    );
    assert_eq!(
        client.token_calls.load(Ordering::SeqCst),
        1,
        "token endpoint should be called once to force-refresh"
    );
    Ok(())
}

/// Credentials endpoint fails 401 twice in a row. After the bounded retry
/// the client must conclude login is really required.
#[test(tokio::test)]
async fn test_credentials_persistent_401_maps_to_login_required() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());
    let host = get_host();
    seed_fresh_tokens(&storage, &paths, &host).await;

    let client = RetryMockClient::new(/*cred_fail=*/ usize::MAX, /*token_fail=*/ 0);
    let result = auth.get_credentials_or_refresh(&client, &host).await;

    assert!(
        matches!(result, Err(Error::Login(LoginError::Required(_)))),
        "expected LoginRequired after persistent 4xx, got: {result:?}"
    );
    assert_eq!(
        client.cred_calls.load(Ordering::SeqCst),
        2,
        "retry must be bounded to one extra attempt"
    );
    Ok(())
}

/// Token endpoint flaps 401 once during the proactive refresh path, then
/// succeeds. The retry must kick in and `get_credentials_or_refresh` must
/// return credentials without surfacing `LoginRequired`.
#[test(tokio::test)]
async fn test_token_refresh_transient_401_recovers() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());
    let host = get_host();

    // Seed *expired* tokens so the proactive refresh path is taken.
    let auth_io = AuthIo::new(storage.clone(), paths.auth_host(&host));
    auth_io
        .write_tokens(&Tokens {
            access_token: "expired-access-token".to_string(),
            refresh_token: REFRESH_TOKEN.to_string(),
            expires_at: chrono::Utc::now() - chrono::Duration::seconds(300),
        })
        .await?;
    auth_io
        .write_client(&OAuthClient {
            client_id: CLIENT_ID.to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
        })
        .await?;

    let client = RetryMockClient::new(/*cred_fail=*/ 0, /*token_fail=*/ 1);
    let creds = auth.get_credentials_or_refresh(&client, &host).await?;

    assert_eq!(creds.access_key, "oauth-access-key");
    assert_eq!(
        client.token_calls.load(Ordering::SeqCst),
        2,
        "token endpoint should be called twice: initial + retry"
    );
    assert_eq!(
        client.cred_calls.load(Ordering::SeqCst),
        1,
        "credentials endpoint should only be called once after successful retry"
    );
    Ok(())
}

/// Synchronization gate used by `CountingCredsClient` to park the
/// `/api/auth/get_credentials` handler mid-call. `entered` signals
/// the test that the handler has been reached; `release` holds the
/// handler until the test lets it return.
#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

/// HTTP client that counts calls to `/api/auth/get_credentials`.
/// Optionally sleeps inside the handler to widen the race window,
/// or parks the handler on a `Gate` for deterministic coordination.
/// Tokens are fresh so no OAuth leg fires.
#[derive(Clone)]
struct CountingCredsClient {
    cred_calls: Arc<std::sync::atomic::AtomicUsize>,
    sleep_ms: u64,
    gate: Option<Arc<Gate>>,
}

#[async_trait]
impl HttpClient for CountingCredsClient {
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        _auth_token: Option<&str>,
    ) -> Res<T> {
        if url.ends_with("/config.json") {
            let body = serde_json::json!({
                "registryUrl": format!("https://{}", get_registry()),
            });
            return Ok(serde_json::from_value(body)?);
        }
        if url.contains("/api/auth/get_credentials") {
            self.cred_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            } else if self.sleep_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
            }
            let body = serde_json::json!({
                "AccessKeyId": "refreshed-key",
                "SecretAccessKey": "refreshed-secret",
                "SessionToken": "refreshed-session",
                "Expiration": (chrono::Utc::now() + chrono::Duration::hours(1))
                    .to_rfc3339(),
            });
            return Ok(serde_json::from_value(body)?);
        }
        panic!("Unexpected GET: {url}");
    }
    async fn head(&self, _: &str) -> Res<HeaderMap> {
        unimplemented!()
    }
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        _: &str,
        _: &HashMap<String, String>,
    ) -> Res<T> {
        unimplemented!("fresh tokens → no OAuth leg fires")
    }
    async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize + Send + Sync>(
        &self,
        _: &str,
        _: &B,
    ) -> Res<T> {
        unimplemented!()
    }
}

async fn seed_expired_creds_fresh_tokens(auth_io: &AuthIo<Arc<MockStorage>>) -> Res {
    auth_io
        .write_credentials(&Credentials {
            access_key: "stale".to_string(),
            secret_key: "stale-secret".to_string(),
            token: "stale-session".to_string(),
            expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
        })
        .await?;
    auth_io
        .write_tokens(&Tokens {
            access_token: ACCESS_TOKEN.to_string(),
            refresh_token: REFRESH_TOKEN.to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await?;
    Ok(())
}

#[test(tokio::test)]
async fn test_auth_refresh_is_single_flight_across_concurrent_callers() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());
    let host = get_host();

    let auth_io = AuthIo::new(storage, paths.auth_host(&host));
    seed_expired_creds_fresh_tokens(&auth_io).await?;

    let client = CountingCredsClient {
        cred_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sleep_ms: 50,
        gate: None,
    };

    let mut handles = Vec::new();
    for _ in 0..10 {
        let auth = auth.clone();
        let client = client.clone();
        let host = host.clone();
        handles.push(tokio::spawn(async move {
            auth.get_credentials_or_refresh(&client, &host).await
        }));
    }

    let mut creds_seen = Vec::new();
    for h in handles {
        creds_seen.push(h.await.unwrap()?);
    }

    assert_eq!(
        client.cred_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "single-flight: 10 concurrent callers must produce exactly one refresh",
    );
    let first = &creds_seen[0];
    for creds in &creds_seen {
        assert_eq!(creds.access_key, first.access_key);
        assert_eq!(creds.expires_at, first.expires_at);
    }
    assert_eq!(first.access_key, "refreshed-key");
    Ok(())
}

#[test(tokio::test)]
async fn test_auth_refresh_lock_is_per_host() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths.clone(), storage.clone());

    let host_a: Host = "a.quilt.dev".parse().unwrap();
    let host_b: Host = "b.quilt.dev".parse().unwrap();

    // Seed each host separately; they live under distinct paths.
    seed_expired_creds_fresh_tokens(&AuthIo::new(storage.clone(), paths.auth_host(&host_a)))
        .await?;
    seed_expired_creds_fresh_tokens(&AuthIo::new(storage.clone(), paths.auth_host(&host_b)))
        .await?;

    // Park host_a's refresh inside the HTTP handler using a gate so
    // it deterministically holds host_a's lock while we exercise
    // host_b. No wall-clock budget — robust under CI load.
    let gate = Arc::new(Gate::default());
    let gated_client = CountingCredsClient {
        cred_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sleep_ms: 0,
        gate: Some(gate.clone()),
    };
    let fast_client = CountingCredsClient {
        cred_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sleep_ms: 0,
        gate: None,
    };

    let auth_clone = auth.clone();
    let client_a = gated_client.clone();
    let host_a_clone = host_a.clone();
    let a_task = tokio::spawn(async move {
        auth_clone
            .get_credentials_or_refresh(&client_a, &host_a_clone)
            .await
    });

    // Wait until host_a is confirmed inside the handler, holding
    // host_a's refresh lock.
    gate.entered.notified().await;

    // Run host_b. If per-host locking works, this completes;
    // otherwise it would block forever on host_a's lock. The
    // timeout is a safety net to fail fast instead of hanging CI.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        auth.get_credentials_or_refresh(&fast_client, &host_b),
    )
    .await
    .expect("host_b refresh must not wait behind host_a's lock")?;

    // Positive assertion: host_a is still parked in its handler,
    // proving host_b completed without host_a making progress.
    assert!(
        !a_task.is_finished(),
        "host_a must still be blocked in its handler while host_b completes",
    );

    // Release host_a so the spawned task can finish cleanly.
    gate.release.notify_one();
    a_task.await.unwrap()?;
    Ok(())
}

#[test(tokio::test)]
async fn test_refresh_lock_map_sweeps_dead_entries() -> Res {
    let storage = Arc::new(MockStorage::default());
    let paths = DomainPaths::new(storage.temp_dir.path().to_path_buf());
    let auth = Auth::new(paths, storage);

    let host: Host = "x.quilt.dev".parse().unwrap();

    // First lookup inserts a live Weak.
    let arc1 = auth.refresh_lock_for(&host);
    assert_eq!(
        auth.refresh_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
    );

    // Dropping all strong refs leaves a dead Weak behind.
    drop(arc1);
    assert!(
        auth.refresh_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&host)
            .expect("entry still present before sweep")
            .upgrade()
            .is_none(),
    );

    // Next lookup sweeps the dead entry and inserts a fresh one;
    // map size stays at 1 instead of accumulating per refresh.
    let _arc2 = auth.refresh_lock_for(&host);
    assert_eq!(
        auth.refresh_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
    );
    Ok(())
}
